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
| calliope-gui (CLI front) | Laptop | Parses arguments before any GUI init; prints `--help` / `--version` | Rust, hand-written parser (no clap) | `src/main.rs` (thin binary), `src/cli.rs` |
| calliope-gui (version) | build time + Laptop | Formats the `YY.MM.BBBB` version; shared by `build.rs` and the crate | Rust, no deps | `src/version.rs`, `build.rs` |
| calliope-gui (GUI shell) | Laptop | Tauri 2 app: main window, plugins, managed state, IPC handler registration | Rust + Tauri 2 (webkit2gtk-4.1 on Linux), `tauri-plugin-window-state` | `src/gui.rs`, `tauri.conf.json` |
| calliope-gui (IPC commands) | Laptop | All `#[tauri::command]` functions; thin wrappers over pure modules. The Player's commands are in their own file | Rust | `src/ipc.rs`, `src/player_ipc.rs` |
| calliope-gui (settings) | Laptop | Loads/saves user settings (theme, repository root, edge-AI server URL, keep-original switch, the last local import folder; MIDI/audio later) as JSON | Rust + serde | `src/settings.rs` |
| calliope-gui (fs helpers) | Laptop | Atomic writes, no-clobber copies, file-name validation, moving to the repository trash, FNV-1a revisions | Rust, std only | `src/fsutil.rs` |
| calliope-gui (track metadata) | Laptop | `track.json` schema (version 2: track type, stems, optional `position` / `album_position` read leniently), validation, v1→v2 migration in memory, `apply_patch` (band/album/positions), ids (UUIDv7), RFC 3339 timestamps | Rust + serde, `uuid` | `src/track_meta.rs` |
| calliope-gui (track repository) | Laptop | Repository layout, scan, save transaction (incl. the main track's positions), delete-to-trash, batch delete (`delete_tracks`: each track on its own, failures reported), revision-checked field patches in two phases (`prepare_patches` refuses the whole batch on any conflict, `commit_patches` writes only changed tracks, one after the other: a track whose write fails is reported in `failed` with its reason and the rest are still written, with no rollback, so a failure can leave mixed old/new positions; the frontend reloads the library after a reported failure so the user sees the real state; `apply_patch` holds only the fields the patch sets to the strict text rules and the rest of the track to the read-level rules, so hand-edited padded names do not block a reorder), export, staged (atomic) creation of new tracks | Rust, pure (no Tauri) | `src/repository.rs` (+ `src/repository_tests.rs`) |
| calliope-gui (file dialogs) | Laptop | Native open/folder/save dialogs opened from Rust (tablatures, repository root, exports, import audio/video/directory); pick tokens; scripted picker for e2e (`e2e-hooks` feature) that also records each request's start folder | Rust + `tauri-plugin-dialog` | `src/picker.rs` |
| calliope-gui (local source paths) | Laptop | Typed/pasted local paths for the import (trim, quotes, `~/`, `file://`), the file and folder checks with exact messages, the audio/video extension lists, the dialog's start folder (`browse_start`: typed path's folder > last import folder > home) and the audio files directly inside a folder (`list_audio_files`, name order) | Rust, pure (std + `url`) | `src/local_source.rs` |
| calliope-gui (import temp space) | Laptop | `<root>/import-tmp/` job folders, resume lookup, safe cleanup | Rust, pure | `src/import_tmp.rs` |
| calliope-gui (external tools) | Laptop | Finds `yt-dlp`/`ffmpeg`/`ffprobe` on `PATH`, version checks; runs them with `calliope_lib::process` | Rust | `src/tools.rs` |
| calliope-gui (media) | Laptop | ffprobe JSON → audio check/duration/tags → `TrackEdits`, track-number tag → album position (`position_from_tags`); ffmpeg → FLAC 44.1 kHz stereo | Rust, pure | `src/media.rs` |
| calliope-gui (download) | Laptop | URL validation/normalisation (`url` crate), yt-dlp args, progress/error parsing, `info.json` mapping | Rust | `src/download.rs` |
| calliope-gui (import job) | Laptop | The single import job: state machine, events, cancel, shutdown; the directory batch (one job in the same slot that runs each file's preparation + extraction in turn, without the edit pane) | Rust, pure (std threads) | `src/import_job.rs`, `src/import_batch.rs` (child module) |
| calliope-gui (stem audio) | Laptop | FLAC stem probing/compatibility rules, decoding into memory as 16-bit for playback, full-precision streaming reader for rendering, the empty-stem decision for imports (`check_stem` over `calliope_lib::flac_level` with early stop at 15 s, `check_reported` for server-reported audible time, `EMPTY_STEM_MIN_AUDIBLE_MS = 15_000`) | Rust, `claxon`, `calliope-lib` | `src/stem_audio.rs` |
| calliope-gui (mixer) | Laptop | Pure mixing shared by playback and Save: dB gains (-60 = Off .. +12), unmuted, mono->stereo, sum, hard clip with clipped-sample count, quantise | Rust, std only | `src/mixer.rs` |
| calliope-gui (transport) | Laptop | Play/pause/stop/seek state machine with the 0.3 s scrub-resume rule; time injected | Rust, pure | `src/transport.rs` |
| calliope-gui (audio output) | Laptop | `OutputBackend` trait: `CpalBackend` (default device), `NullBackend` (paced, optional WAV capture; the only backend in `e2e-hooks` builds), `ManualBackend` (unit tests) | Rust, `cpal` (ALSA on Linux) | `src/audio_out.rs` |
| calliope-gui (editor engine) | Laptop | Editor sessions: load stems, lock-free render callback, transport, transient solo, clip indicator, events, remembered mixes, the backing save job, the mix waveform (0.3 s debounce, worker thread), the session's tempo map, the beat-detection job's start/cancel/outcome | Rust (std threads) | `src/editor.rs` (+ `src/editor_tests.rs`) |
| calliope-gui (mix waveform) | Laptop | `mix_peaks` (2048 max-abs buckets of the checked stems' mix, the playback mixer's maths, cancellable) and `Debounce` (restartable 300 ms deadline, injected `Instant`s) | Rust, pure | `src/mix_waveform.rs` |
| calliope-gui (tempo detection) | Laptop | Beats + downbeats (from the edge-AI server) → tempo sections: glitch repair, optimal segmentation (sections ≥ 1 bar, > 4 % apart), absorbing glitch sections and folding 2x/0.5x octave errors into the neighbour's tempo, BPM per section (0.1), anchor on the first downbeat, each change snapped to the previous section's 1/64-beat step and recorded as detected | Rust, pure | `src/tempo_detect.rs` |
| calliope-gui (tempo store) | Laptop | `TempoStore` (load/replace a track's tempo map): `FileTempoStore` on `player.json`; the app's `PlayerTempoStore` works under the `PlayerHost` lock: `load` flushes an open Player session first, `replace` flushes and closes it (`PlayerManager::release`) before writing the file | Rust | `src/tempo_store.rs`, `src/player_ipc.rs` |
| calliope-gui (beat detection job) | Laptop | Renders the drums + bass + guitar stems (`other` without a guitar, else all stems) at unity gain to a part file, uploads it to `POST /v1/beats`, waits, fetches and validates the beats, derives the sections, saves them through the `TempoStore`; phases, cancel, cleanup | Rust (own std thread) | `src/beat_detect.rs` (+ `src/beat_detect_tests.rs`) |
| calliope-gui (backing render) | Laptop | Offline render of the mix to FLAC | Rust, `flacenc` | `src/backing_render.rs` |
| calliope-gui (tempo map) | Laptop | Tempo segments, beat grid in 1/64-note ticks (16 per beat, `beats_per_bar` default 4), ms<->tick, snapping, "set BPM at position", beats in a range, loop boundary re-snapping; detected changes (`Segment.detected_ms`) and placed changes (`Segment.on_beat`, added or moved in the Player) both start their section on a beat: beat-rounded tick base, stretched last beat. Section operations: `move_change` (1/64-beat steps of the left tempo; detected: ±256 from the detected position, widened to the current offset; others: counted from the left section's start, no limit), `set_section_bpm`, `scale_section` (×2/÷2), `add_change` (on the split section's 1/64-beat grid), `remove_change`; every section ≥ 1 beat of its own tempo; `set_change_offset` (the Editor's, detected only). Per-section settings `Segment.meta: SectionMeta {name, metronome, count_in}` (flattened; carried through edits: Add copies the split section's metronome flag, Remove drops the joined section's), `set_section_flags`, `set_section_name`, `section_jump_ms` (start, or end; the player lands End on the last frame inside the section) | Rust, pure | `src/tempo_map.rs` |
| calliope-gui (player state) | Laptop | `player.json` schema 6 (reads 1-6, migrating the old global metronome of 1-4; lenient read, strict write, newer schema read-only), the track speed (stored as a factor 0.25..2.0, used as a whole percentage), loop list operations (a new loop ends at the next tempo section or the track end), per-loop speed-training settings (Training on also sets Loop On; Loop On off ends Training), source choice rule, load/save | Rust + serde, pure | `src/player_state.rs` |
| calliope-gui (speed rules) | Laptop | Speed percentages (25..200, `-`/`+` to the next multiple of 5, typed whole numbers), factor ↔ percent, speed-training settings (`Training`: on, start, end, step, repeats; validation) and their pass schedule (`pass_speed`, `progress`) | Rust, pure | `src/speed.rs` |
| calliope-gui (time-stretch) | Laptop | `Stretcher`: real-time time-stretch with the pitch kept (stereo f32), latencies, the alignment rule (seek pre-roll ending at target + input latency, discard the output latency); no allocation after `new` | Rust over `signalsmith-stretch` (MIT; vendored Signalsmith Stretch C++, MIT) | `src/stretch.rs` |
| calliope-gui (player sources) | Laptop | Dropdown entries of a track (backing variants, `audio`, `original`, stems), decoding one into memory (FLAC: `claxon`; other formats: external `ffmpeg` piped to memory), waveform peaks (2048) | Rust | `src/player_source.rs` |
| calliope-gui (metronome) | Laptop | Synthesised click (own sound), count-in plan (at `bpm × speed`), placing clicks in an output block, `section_clicks` (the song's click list: beats of sections whose metronome is on, with each section count-in replacing the clicks before that section; skipped at the end of a repeating loop) | Rust, pure | `src/metronome.rs` |
| calliope-gui (player engine) | Laptop | Player sessions: one decoded source, a callback-owned `Engine` (playback, count-in, loop repeats, metronome, fades; bypass mode at 100 % and stretch mode otherwise: song-time timeline delayed by the stretcher latency, clicks and wraps placed at the heard song position, crossfade into stretch mode; the speed-training run with per-pass speeds switched at the loop start) fed by a bounded command channel, the track speed (`speed` actions, refused during a training run), transport + scrub rule, events, debounced `player.json` writes, `edit_sections(id, SectionEdit)` (Player tempo-section edits, section flags and names applied live: new click grid, loops re-snapped, debounced write; stale-index guard), `jump_section` (instant jump to a section's start or last bar), the grid sent to the engine is `section_clicks` (re-sent when the repeating loop changes; the engine's metronome flag only means "has a tempo" since gui-player-tempo-sections phase 3), the Play pre-roll comes from the Count In of the section under the playhead and loop Play's from the loop's Count-In, `release(id)` (flush + close + `closed` event when the Editor writes the tempo), `finish_open` re-reads a `player.json` changed since its off-lock load | Rust (std threads) | `src/player.rs` (+ `src/player_tests.rs`) |
| calliope-gui (frontend) | Laptop (embedded webview) | Navigation shell, views (Library: track tree (album / track order, "3. Title" labels; Library only: pointer drag and drop and Alt+Up/Down reordering, a Delete icon button and the Delete key for tracks and albums) + track pane (read-only Position; band/album edits rename the whole band/album after a confirmation, planned by `library-rename.ts`); Import: stem extraction steps; Editor: shared track tree + stem mixer + Mix lane with the mix waveform (tempo sections, draggable changes) and Autodetect BPM; Player: shared track tree (Ctrl+L rail) + source dropdown, tablature placeholder, canvas waveform (loops + editable tempo sections: drag, keys, BPM label overlay), under the waveform two full-width rows: Sections (the BPM group, then status + section name box, Add, Remove, ×2, ÷2, Start, End and the section's Click / Count In toggles; no global metronome) beside Loops (a "Training…" button on its third line opens the speed-training popover; the training progress shows in its status line; Loops wraps below in narrow windows), then Playback at the bottom (transport, then the Speed group: `-`, % box, `+`, Reset and the played-BPM readout, wrapping to a second line in narrow panes); Home/End and `[`/`]` keys); Settings), theme, keyboard shortcuts | Svelte 5 + TypeScript + Vite 8, shadcn-svelte (bits-ui, Tailwind v4), Inter font; built to `dist/` and embedded at compile time | `src/ui/` |
| calliope-lib (shared lib) | Laptop + archserver | "calliope-stems API v1" types, validation (incl. the beats document, `check_beats`), FLAC STREAMINFO parser, FLAC audible-time scan (`flac_level`: 100 ms windows, RMS of the loudest channel above `AUDIBLE_LEVEL_DBFS = -40`, integer sum-of-squares comparison, peak for display; no 15 s decision), HTTP client (feature `client`), child-process runner (argv only, process group, PDEATHSIG, cancel). No Tauri/GTK | Rust; `serde`, `libc`, `claxon`, `ureq` 3 without TLS (feature) | `src/calliope-lib/` |
| calliope-stems (edge-AI stems service) | archserver (LAN), deployed by hand as a systemd user unit | HTTP server for "calliope-stems API v1": validates FLAC uploads (≤ 15 min), queues jobs (1 running), runs a configurable separator command, measures each stem's audible time, serves the stems, cleans up; beat jobs (`POST /v1/beats`) run a configurable beat-tracker command in the same queue; `--measure FILE...` prints audible times without starting a server. No Tauri/GTK; the GUI doesn't depend on it | Rust; `tiny_http`, `calliope-lib` | `src/calliope-stems/` |
| separator adapter | archserver | `<sep> <input.flac> <out_dir> <model>` → `<out_dir>/<stem>.flac`; the real one runs the owner's audio-separator venv with `htdemucs_6s` (frees VRAM from Ollama first) | bash | `src/calliope-stems/separators/audio-separator.sh` |
| yt-dlp, ffmpeg, ffprobe | Laptop | External executables, installed by the user, never bundled | upstream | system `PATH` |
| audio-separator, Demucs, PyTorch | archserver | The real separation model, the owner's existing install in `~/edge-ai/stems/.venv`; never bundled, never run by tests | upstream (Python) | outside this repo |
| beat-tracker adapter | archserver | `<tracker> <input.flac> <out_dir>` → `<out_dir>/beats.json`; the real one runs Beat This! (`Audio2Beats`, checkpoint `final0`, CUDA with CPU fallback) from the beats venv | bash + embedded Python | `src/calliope-stems/trackers/beat-this.sh` |
| Beat This! (beat-this 1.1.0, MIT code and weights), PyTorch, soundfile | archserver | The beat/downbeat model, the owner's install in its own venv `~/edge-ai/beats/.venv`, checkpoint `~/edge-ai/beats/models/final0.ckpt`; never bundled, never run by tests | upstream (Python) | outside this repo |
| test stand-ins | dev machine | `tests/support/stub-separator` (canned stems, failure modes) and `tests/support/stub-beat-tracker` (beats computed from the input's duration: two tempos 120→90, steady, failure modes) used with the real `calliope-stems` on 127.0.0.1; `tests/support/bin/yt-dlp` (fake downloader) | bash / Python 3 stdlib | `tests/support/` |

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
    `save_track(request) -> SaveResult` (request may carry `position`, `album_position` and `group: [TrackPatch]`, the other tracks of a band/album rename, checked before and written after the main save; the result then has `updated` and `failed`), `delete_track(id, revision)`,
    `update_tracks(patches: [TrackPatch{id, revision, band?, album?, position?, album_position?}]) (position fields: absent = unchanged, JSON `null` = clear, a number 1..9999 = set; the same for `save_track`'s `position`/`album_position`) -> UpdateResult{tracks, failed: [{id, title, message}]}`,
    `delete_tracks(items: [{id, revision}]) -> DeleteResult{deleted: [id], failed}` (gui-library-album-order plan §2.4-2.5),
    `export_track(id) -> Option<String>`, `export_tablature(id, name) -> Option<String>`;
    edge-AI and import (gui-stem-extraction plan §2.10): `set_edge_ai_url(url)`, `set_keep_original(keep)`,
    `check_edge_ai()`, `check_tools()`, `prepare_url_import(url)`,
    `start_url_import(url, resume, events)`, `pick_import_file(kind: audio|video|directory, typed) -> Option<PickedFile{path, name}>`
    (the start folder comes from `typed`, the last import folder or home; gui-import-local-batch plan §2.3),
    `import_path(kind, path, events)` (gui-existing-track-import plan §2.4), `import_directory(path, events)` (a batch job,
    gui-import-local-batch plan §2.4-2.5),
    `start_stem_extraction(job, edits)`, `cancel_import(job)`, `discard_import(job)`,
    `get_import_job()`, `watch_import(events)`; editor (gui-backing-track-editor plan §2.7):
    `open_editor(id, events)`, `close_editor()`, `get_editor()`, `watch_editor(events)`, `editor_play(id)`,
    `editor_lane_play(id, name)` (check + solo + play), `editor_end_solo(id)`, `editor_pause(id)`,
    `editor_stop(id)`, `editor_seek(id, position)`, `editor_nudge(id, delta)` (ms),
    `editor_set_stem(id, name, gain, unmuted)` (gain in dB or null = Off), `save_backing(id)`,
    `cancel_backing_save(id)`, beat detection and tempo (stems-beat-autodetect plan §2.11): `editor_detect(id)` (renders the
    drums, bass and guitar stems at unity gain and starts the detection job), `editor_cancel_detect(id)`, `editor_tempo(id, action)` (`move` {index, steps: absolute
    offset from the detected position, -256..256} | `reload`); player (gui-play-backing-track plan §2.9, in `src/player_ipc.rs`):
    `open_player(id, events)`, `close_player()`, `get_player()`, `watch_player(events)`,
    `player_select_source(id, source)` (stops playback, keeps the position), `player_play(id)`, `player_pause(id)`, `player_seek(id, position)`,
    `player_nudge(id, delta)` (both with the 0.3 s scrub rule), `player_skip(id, delta)` (instant, Rew/FF),
    `player_leave(id)` (pause + close output + flush), `player_tempo(id, action)` (`set-bpm` | `speed-step` {steps} |
    `speed-reset` | `speed-set` {percent} (gui-player-speed plan §2.8; refused during a training run) | the section edits of gui-player-tempo-sections plan §2.5: `section-bpm` {index, at, bpm}, `scale-section`
    {index, at, factor: double|halve}, `add-change` {position, bpm}, `remove-change` {index, at}, `move-change` {index, at,
    steps}; phase 2: `section-flags` {index, at, metronome, count_in}, `section-name` {index, at, name}, `jump-section`
    {index, at, edge: start|end}; `at` = the section's current `start_ms`, a mismatch is refused as stale), `player_loop(id, action)` (tagged action: add, remove,
    select, mark, set, flags, training {on, start, end, step, repeats}, play, stop; loops may have no end yet). The
    Player transport carries `bpm` (song BPM at the playhead), `played_bpm` (× the speed heard), `speed_pct` (the speed
    heard: the training speed during a run) and `training` (`{loop, speed_pct, repeat, of|null}` or null); the snapshot
    carries the track speed `speed_pct` and each loop's `training`. Actions are tagged objects to keep the command list
    short. BPM detection runs from the Editor (Autodetect BPM, stems-beat-autodetect); the Player has no Detect button.
    Types are in `src/ui/lib/ipc.ts` and in the plans (tracks-repository §2.7, stem-extraction §2.10,
    backing-track-editor §2.7, play-backing-track §2.9, stems-beat-autodetect §2.11).
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
    Commands that consume a choice take the token. **One exception**: the import's local-file
    source is a path box (typed, pasted or filled by Browse via `pick_import_file`, which returns
    the full path for display), and `import_path(kind, path, events)` takes that text. It only
    reads: `local_source::resolve` requires an absolute path to a regular, readable file with an
    audio/video extension, and the file then goes through ffprobe/ffmpeg exactly as before.
    `import_directory(path, events)` (gui-import-local-batch) is the same exception for a folder:
    `local_source::resolve_dir` + `list_audio_files` read the folder's direct entries only, and each
    audio file goes through the same read-only preparation. `pick_import_file`'s `typed` text is only
    used to choose the dialog's start folder (metadata only). No other command accepts a path, and
    no path from the frontend is ever written, created or deleted. Track ids and file names sent by the
    frontend are validated and resolved strictly inside `<root>/tracks/<id>/`.
  - The frontend never keeps time. Audio/MIDI/sync logic stays in Rust; the frontend displays
    state and sends commands. Audio is played by Rust (`cpal`), never by the webview (no Web
    Audio, no `<audio>`, so the CSP needs no `media-src`/`blob:`). Playback position reaches the UI
    as `transport` events on the editor/player channel (on every change, else every 100 ms while playing).
    Only one engine sounds at a time: Player Play stops the Editor and Editor Play pauses the Player. Both
    share one `Arc<dyn OutputBackend>` from `select_backend()`.
- **Frontend log lines** (stderr, prefix `calliope-ui: `), used by the GUI tests:
  `ready view=<id> theme=<t> version=<v>`, `view=<id>`, `theme=<t>`,
  `csp-violation directive=<d> blocked=<uri>`, `error <context>: <msg>`; Library:
  `library root=<p> status=<s> tracks=<n> problems=<m>`, `select id=<id>`, `mode=edit|view id=<id>`,
  `saved id=<id> tablatures=<n> warnings=<w>`, `deleted id=<id>`, `exported id=<id>`,
  `tab-staged <add|update|remove> name=<n>`, `repository root=<p> default=<bool>`; order and delete (gui-library-album-order):
  `reorder tracks album=<key> id=<id> from=<i> to=<j> via=<drag|key> written=<n> failed=<k>`,
  `reorder albums band=<key> album=<key> from=<i> to=<j> via=<drag|key> written=<n> failed=<k>`,
  `reorder none key=<k> via=<drag|key> reason=<search|edit|busy|band|no-album|first|last|same|outside>`, `drag start key=<k>`,
  `drag target key=<k> gap=<g|none>`, `rename kind=<band|album> from="<a>" to="<b>" tracks=<n> merged=<b> failed=<k>`,
  `rename cancelled`, `rename refused reason=<clear-band|clear-album|both>`, `deleted album band="<b>" album="<a>" tracks=<n> failed=<k>`,
  `delete cancelled key=<k>`, `delete none key=<k> reason=<band|no-album|none|edit|busy>`.
  Import: `import mode=stem-extraction`, `import source=url|audio|video|directory`, `import path-picked kind=<audio|video|directory>`,
  `import phase=<phase> job=<job>`, `import error stage=<s> message=<m>`,
  `import saved id=<id> stems=<n> original=<b> dropped=<a,b|none>`; directory batch: `import batch file=<i>/<n> name=<name>`,
  `import batch item=<i> result=saved id=<id> dropped=<..>|failed stage=<s> message=<m>`, `import batch finished saved=<a> failed=<b>`,
  `import batch cancelled saved=<a> of=<n>`.
  Editor: `editor active id=<id> stems=<n>`, `editor inactive id=<id|none>`,
  `editor play|pause|stop position=<ms>`, `editor solo name=<n|none>`, `editor leave stop`,
  `editor seek position=<ms>`, `editor nudge delta=<ms>`,
  `editor stem name=<n> gain=<db|off> unmuted=<b>`, `editor saved id=<id> file=<f> clipped=<n>`,
  `editor waveform peaks=<count> max=<0..1>`, `editor detect start|cancel`, `editor detect phase=<p>`,
  `editor detected segments=<k> bpm=<a,b,..>`, `editor detect error message=<m>`,
  `editor tempo move index=<i> steps=<s> applied=<s'>`, `editor tempo reload segments=<k>`, `player reopen id=<id>`.
  Player: `player active id=<id> source=<key> sources=<n> mode=<bpm|time> loops=<n> position=<ms>`, `player inactive id=<id|none>`,
  `player source key=<key>`, `player play|pause position=<ms>`, `player seek position=<ms>`, `player skip delta=<ms>`,
  `player bpm value=<v> position=<ms>`,
  `player loop <action> index=<n|none> count=<n> complete=<b>`, `player tree collapsed=<b>`, `player leave pause`,
  `player section <bpm|scale|add|remove|move> index=<i> …`, `player section flags index=<i> metronome=<b> count_in=<b>`,
  `player section name index=<i> length=<n>`, `player section jump index=<i> edge=<start|end> position=<ms>`,
  `player speed percent=<n> via=<minus|plus|reset|typed|key>`, `player speed refused text=<t>`,
  `player training index=<i> on=<b> start=<s> end=<e> step=<t> repeats=<r>`, `player readout bpm=<text|none> speed=<pct>`
  (`player active` ends with ` speed=<pct>`),
  `player sections segments=<k> bpm=<a,b,..> starts=<ms,..> metronome=<1|0,..> count_in=<1|0,..> named=<n>`.
  Rust lines (prefix `calliope: `): `dialog kind=<add-tablature|update-tablature|repository-root|export-track|export-tablature|import-audio|import-video|import-directory> result=<picked|cancelled>`,
  `dialog start kind=<import-audio|import-video|import-directory> from=<typed|last|home|none> dir=<p|none>`, `import last-location dir=<p>`, `import last-location forgotten`,
  `import job=<job> phase=<phase>`, `import path kind=<audio|video> result=<accepted|rejected>[ reason=<empty|invalid|relative|missing|folder|not-file|unreadable|extension>]`,
  `import path kind=directory result=accepted files=<n>|rejected reason=<empty|invalid|relative|missing|unreadable|not-folder|no-audio>`,
  `import batch job=<j> files=<n>`, `import batch job=<j> file=<i>/<n> name=<name> result=started|saved id=<id> dropped=<..>|failed stage=<s> message=<m>|cancelled`,
  `import batch job=<j> finished saved=<a> failed=<b>`, `import batch job=<j> cancelled saved=<a> skipped=<k>`,
  `tool <name> found=<bool> version=<v>`, `track deleted id=<id> trash=<folder>`, `track delete failed id=<id> message=<m>`,
  `tracks deleted deleted=<n> failed=<k>`, `tracks updated written=<n> unchanged=<u> failed=<k>`, `track update failed id=<id> message=<m>`,
  `audio backend=<cpal|null|capture> rate=<hz>`, `editor open id=<id> stems=<n> duration_ms=<ms>`,
  `editor transport playing=<b> audible=<b> position_ms=<ms> solo=<n|none> clipping=<b>` (changes only),
  `editor saved id=<id> file=<f> bits=<b> clipped=<n>`, `editor peaks id=<id> gen=<n> ms=<ms>`,
  `editor detect id=<id> phase=<checking|mixing|uploading|waiting|detecting|applying>`,
  `editor detected id=<id> beats=<n> downbeats=<m> bar_beats=<b> sigma_ms=<s> segments=<k> bpm=<..> starts_ms=<..>`,
  `editor detect failed id=<id> message=<m>`, `editor detect cancelled id=<id>`, `player released id=<id> flushed=<b>`,
  `tempo saved id=<id> segments=<k> detected=<n>`, `player open id=<id> source=<key> duration_ms=<ms> rate=<hz> position_ms=<ms>`,
  `player transport playing=<b> counting_in=<b> loop=<n|none> position_ms=<ms>` (changes only),
  `player count-in clicks=3 beat_ms=<ms>`, `player wrap loop=<index, 0-based> count_in=<b>`, `player state saved id=<id>`,
  `player state error id=<id> message=<m>`, `player section <bpm|scale|add|remove|move|flags|name> id=<id> index=<i> segments=<k> bpm=<..>[ steps=<s> applied=<s'>] metronome=<1|0,..> count_in=<1|0,..>`,
  `player section jump id=<id> index=<i> edge=<start|end> position_ms=<ms>`, `player speed id=<id> percent=<n>`,
  `player training loop=<i> pass=<n> speed=<pct> repeat=<r>/<of|end>`, `player training end loop=<i> speed=<pct>`.
- **Settings file**: `$XDG_CONFIG_HOME/app.calliope.gui/settings.json` (Tauri
  `app_config_dir()`), e.g. `{"theme": "dark", "repository_root": null, "edge_ai_url": null, "keep_original": false, "last_import_dir": "/music/rock"}`
  (`repository_root`: `null`/absent = the default root, must be an absolute path;
  `edge_ai_url`: `null` = not configured, else `http://host[:port][/path]`, no TLS/userinfo/query;
  `keep_original`: keep the original full mix as `original.flac` with imported stems;
  `last_import_dir`: `null` or an absolute path, the folder of the last accepted local import (a file's folder; an imported
  directory itself), shared by audio, video and directory Browse, cleared when Browse finds it missing). Missing or invalid files give the defaults;
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
                                  (an import leaves out empty stems, so often fewer than the model's 6)
      original.flac               the full mix, only when kept at import (setting)
      backings/<variant-id>.flac  backing-track variants of a stem track made in the Editor (listed in
                                  `backings`); this feature writes `backings/backing.flac` (`backing-2`..
                                  if a user file already has the name)
      .calliope-backing-<uuid>.part  a backing being rendered (removed on success/failure/cancel)
      player.json                 the Player's settings for this track (reserved name; see below)
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
  `track.json` fields: `schema_version, id, type ("backing"|"stem"), band, album, album_position?, title, position?,
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
  **Order** (gui-library-album-order plan §2.2): `position` = the track's place in its album and `album_position` = its album's
  place among the band's albums (1..9999, omitted when unset; additive, no schema bump, so schema-2 builds keep them in
  unknown fields). Read leniently (any other value = unset, never a problem), validated on write. The album's place is the
  minimum `album_position` of its tracks (grouping = the tree's trimmed, case- and accent-insensitive names); an album reorder
  writes it to every track of the band's albums. Tracks of "(no album)" ignore `position`. Imports set `position` from the
  `track` / `tracknumber` tag ("3", "3/12") when the track has an album, never `album_position`, and never touch other tracks.
  Ordered tracks/albums come first (by number, then name), then the others alphabetically.
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
- **Player settings file `player.json`** (gui-play-backing-track plan §2.3-2.5), next to `track.json`:
  `{"schema_version": 6, "source": "backing:<id>"|"audio"|"original"|"stem:<name>", "position_ms": <int>, "tempo": {"beats_per_bar": 4,
  "segments": [{"start_ms": <number>, "bpm": <20..300>[, "detected_ms": <number>][, "on_beat": true][, "name": <string>][, "metronome": false][, "count_in": true]}]}, "speed": <0.25..2.0>,
  "loops": [{"id", "start": {"tick": <int>}|{"ms": <number>}, "end": <same>|null, "repeat", "count_in"[, "training": {"on", "start", "end", "step", "repeats"}]}], "current_loop": <id|null>}`.
  `end: null` = a loop whose end is not set (no repeat, Play disabled, only its start mark drawn). Since gui-player-tempo-sections
  phase 3, `+` sets the end at once: the next tempo section's start tick after the loop start, else the track end (ms in
  timestamp mode); `end` is null only when there is no room, or after `>` moved the start past the end.
  The playback position is saved per track (on pause/seek/leave/switch/exit and every 10 s while playing).
  No segments = BPM 0 = timestamp mode; loop boundaries are ticks (1/64 notes from the first segment's start) in BPM
  mode and ms in timestamp mode, re-snapped (keeping their time) whenever the tempo map changes. One state per track,
  shared by all sources (stems and backings are sample-aligned). Written with `write_atomic` 500 ms after the last
  change and at once on track switch, view leave, close and exit; never creates the track folder. Lenient on read
  (bad parts dropped, unknown fields kept), a newer `schema_version` is never overwritten. Library scans ignore it,
  Export copies it, Delete moves it with the folder. `player.json` is reserved as a file name like `track.json`
  (`fsutil::validate_file_name`).
  **Schema 2** (stems-beat-autodetect plan §2.3): a segment may carry `detected_ms`, the position beat detection gave that
  tempo change (never on the first segment). Such a *detected change* starts its section on a beat: its tick base is rounded
  to the nearest whole beat of the global count and the 16 ticks of the left section's last beat are spread linearly up to
  `start_ms` (a stretched beat of 1/2..3/2 beats); it moves in 1/64-beat steps of the left tempo, at most ±256 steps from
  `detected_ms`. Typed changes keep the tick rule; loops still snap to 1/64 notes. Schema 1 files are read as before
  (they have no detected changes); writes are schema 5 since gui-player-tempo-sections phase 3 (below), so older builds treat the file as newer and never overwrite it.
  Typing a BPM at a detected change's start, or anywhere inside the stretched last beat before it, replaces that change's
  BPM and keeps its position and `detected_ms` (so the tick bases stay strictly increasing; `bases()` also clamps them as a
  safety net); removing or merging a segment drops it; a new
  detection replaces the whole map.
  **Schema 3** (gui-player-tempo-sections plan §2.3): a segment may carry `"on_beat": true`, a *placed change* (added in the
  Player, or a typed change once moved there). It starts its section on a beat exactly like a detected change (same base
  and stretch rules), but has no detected position and no move limit; it moves in 1/64-beat steps of the left tempo counted
  from the left section's start. `on_beat` is read only as JSON `true` and never on the first segment. This build reads
  1-3 and wrote 3 (schema 4 since phase 2, below); schema-2 builds treat the file as newer and never overwrite it (they would otherwise drop `on_beat`
  and re-round the bars). Player section edits (BPM of one section, ×2/÷2, add, remove, move) go through the open Player
  session (no release): the engine gets the new grid at once (the metronome follows from the next beat, playback is not
  interrupted), loops are re-snapped keeping their time, and the file is written 500 ms later.
  **Schema 4** (gui-player-tempo-sections plan §10.2): a segment may carry per-section settings, written only when not the
  default: `name` (trimmed, no control characters, ≤ 40 characters), `"metronome": false` (this section doesn't click) and
  `"count_in": true` (three clicks at this section's tempo replace the previous section's last clicks, never on the first
  segment: schema 4 only, superseded by schema 5 below). In schema 4 the top-level `metronome.on` was the master switch for all clicks and `metronome.count_in` the
  Play / loop pre-roll. Schema 1-3 files read with every section clicking, no count-in and no names; schema-3 builds
  treat schema-4 files as newer and never overwrite them (their parser would drop the keys).
  **Schema 5** (gui-player-tempo-sections plan §18.3, owner decisions 2026-10-18): there is no global metronome;
  the top-level `metronome` object is no longer written. A section clicks only when its `metronome` is not false, and
  `count_in` is allowed on the first segment, where it means only the Play pre-roll. Play counts in when the section under
  the playhead has `count_in`; loop Play only with the loop's `count_in`. Schema 1-4 files are migrated on read: master
  `on` false (or missing) → every section `metronome: false` and `count_in: false`; master `on` true → the section flags
  kept; then master `count_in` true → the first section's `count_in: true`. A v<=4 file with an empty tempo map has nothing to
  migrate, so its old master state is dropped. Schema-4 builds treat the file as newer and never overwrite it.
  **Schema 6** (gui-player-speed plan §2.7): `speed` is the track speed as a factor of a whole percentage, 0.25..2.0
  (read as `round(speed × 100)` clamped to 25..200 %, so older files, always 1.0, open at 100 %); a loop may carry
  `training` (`on`, `start`/`end` 25..200, `step` 1..175, `repeats` 1..99, whole percentages), written once set, dropped
  as a whole when damaged. Schema 6 because a schema-5 build clamps `speed` to 0.5..1.5 and would write it back. This
  build reads 1-6 and writes 6; schema-5 builds treat the file as newer and never overwrite it.
  **The track's one tempo map** (stems-beat-autodetect plan §2.2): `tempo` in `player.json` is also written by the Editor
  (Autodetect BPM, moving a detected change) through `TempoStore::replace`, entirely under the `PlayerHost` lock: if the
  Player holds the track, its pending state is flushed and its session closed (`PlayerEvent::Closed`; the Player view
  reopens the track when it is shown), then the file is loaded → tempo replaced (loops re-snapped keeping their time,
  speed, position and other fields kept; until gui-player-speed `speed` was reset to 1.0) → `write_atomic`. A failed flush refuses the write. The Player's
  `finish_open` re-reads a `player.json` whose revision changed since its off-lock load, so neither side loses the
  other's write. The Editor reads the map through the store (flushing an open Player session) at open and on each Editor
  view entry.
- **Edge-AI stems protocol ("calliope-stems API v1")**, types and client in
  `calliope-lib` (`stems_api`, `stems_client`), server `calliope-stems`; plain HTTP on the
  LAN, base URL from settings (e.g. `http://archserver:8765`); full table in the
  gui-stem-extraction plan §2.7:
  `GET /v1/health` → `{"service":"calliope-stems","api":1,"version","models":[...],"default_model":"htdemucs_6s","busy","max_duration_s":900,"max_upload_bytes"}`;
  `POST /v1/jobs?model=<m>` (body = FLAC, `Content-Type: audio/flac`, `Content-Length`) →
  `202 {"job","state":"queued"}` (400 model, 411 no length, 413 size/duration, 415 not FLAC,
  503 queue full);
  `GET /v1/jobs/<id>` → `{"job","state":"queued|running|done|failed|cancelled","progress","stems","error"[,"stem_levels"]}` (404 unknown);
  `stem_levels` (additive, gui-stem-extraction-active-time plan §2.2; it replaced the never-released
  `stem_peaks`, which clients ignore if a server-peaks build still sends it) is present only when `done`
  and measured: `[{"name","audible_ms" (integer, multiple of window_ms),"level_dbfs" (integer, -40),
  "window_ms" (100),"peak_dbfs" (display only, null = digital silence)}]`; a stem without an entry was
  not measured; no key = an older server or `--no-stem-levels`. The client validates it only on `done`
  (`stems_api::check_stem_levels`; names in `stems`, unique, window_ms 1..=1000, level -150..=0,
  audible_ms ≤ 24 h and a multiple of window_ms; otherwise "bad stem level list") and uses an entry
  only when its level and window equal its own `flac_level` constants;
  `GET /v1/jobs/<id>/stems/<name>` → FLAC (409 not done); `DELETE /v1/jobs/<id>` → 204 (cancel + cleanup).
  **Beats (additive, stems-beat-autodetect plan §2.4):** health gains `"tasks": ["stems"]` or `["stems","beats"]` (absent =
  stems only); `POST /v1/beats` (FLAC body, same checks as jobs; 501 without a tracker) → `202 {"job","state"}`; the job's
  status has `"kind":"beats"` (stem jobs have no `kind` key, unchanged JSON) and `"stems": null`;
  `GET /v1/jobs/<id>/beats` → `{"beats":[s..],"downbeats":[s..],"tracker"}` (409 not done, 404 unknown or not a beats job);
  `stems/<n>` of a beats job → 404. Beats document (`stems_api::check_beats`, on server and client): seconds, finite,
  0..duration+1 s, strictly increasing, ≤ 20 000 beats, each downbeat equals a beat within 1e-6 s, `tracker` ≤ 100 chars,
  JSON ≤ 1 MiB. Beat jobs share the queue and the single worker with separation jobs. The app derives tempo sections
  itself (`tempo_detect`); the server only reports beats.
  Errors are `{"error": "..."}`. **No auth** (LAN only, owner-approved default). Client:
  connect 5 s / request 30 s timeouts, 1 s polling, job id and stem names validated, ≤ 16
  stems, ≤ 1 GiB each, FLAC magic checked. "running" = the server confirmed processing start
  (UI shows "Working..."). A protocol conformance suite (`src/calliope-stems/tests/conformance.rs`)
  pins it. The protocol never promises a stem count. The GUI decides which stems are empty
  (`audible_ms < stem_audio::EMPTY_STEM_MIN_AUDIBLE_MS`, 15 s): stems the server reports empty are not
  fetched; every fetched stem is measured locally (early stop at 15 s; local emptiness wins)
  (active-time plan §2.4; logs carry `audible_s=…`, `source=server|local` and
  `server_levels=<n>/<total>`): `ImportEvent::Saved` and `JobSnapshot` carry
  `dropped: [{name, audible_ms}]`; the finished page says "Dropped empty stems: …"; an all-empty result
  fails the import (stage `server`, "Every stem is empty (less than 15 s above -40 dBFS), so no track
  was saved").
- **`calliope-stems` command line**: `--listen ADDR:PORT` (default `0.0.0.0:8765`; prints
  `calliope-stems listening addr=…` on stderr), `--work-dir` (default
  `~/.local/state/calliope-stems`), `--separator PATH` (**required**, so nothing starts the real
  model by accident), `--model` (`htdemucs_6s`), `--max-upload-mb` (300), `--max-duration-s`
  (900), `--queue` (2 waiting; 1 running, fixed), `--separator-timeout-min` (30),
  `--retention-hours` (24), `--no-stem-levels` (don't measure or report `stem_levels`), `--beat-tracker PATH` (optional;
  enables `POST /v1/beats`; a missing or non-executable path logs a warning and runs with beats off, a missing value is an error), `--beat-timeout-min` (10; hidden test knob `--beat-timeout-ms`, min 100); `--measure FILE...` (diagnostic: print each FLAC file's `audible_ms`/`windows`/`peak_dbfs` and exit, no `--separator` needed). Jobs live in memory; job folders `<work>/jobs/<uuid>/` carry a
  marker and are the only things it deletes. Logs on stderr (journald), prefix `calliope-stems: `.
  **Separator contract**: `<separator> <input.flac> <out_dir> <model>`, writes
  `<out_dir>/<stem>.flac`, optional stdout `progress <0..1>`, exit 0; outputs are validated.
  **Beat-tracker contract**: `<tracker> <input.flac> <out_dir>`, writes `<out_dir>/beats.json` (the beats document), optional
  `progress <0..1>` lines, exit 0; validated with `check_beats`; killed (process group) on cancel, timeout and server exit.
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
    src/lib.rs            declares all modules (library crate `calliope_gui`); main.rs is the thin binary
    src/*.rs              Rust modules (cli, version, gui, ipc, settings, fsutil,
                          track_meta, repository (+ repository_tests), picker, local_source, import_tmp,
                          tools, media, download, import_job (+ import_batch), stem_audio, mixer, transport,
                          audio_out, editor (+ editor_tests), backing_render, tempo_map,
                          player_state, player_source, metronome, player (+ player_tests),
                          player_ipc, mix_waveform, tempo_detect, tempo_store,
                          beat_detect (+ beat_detect_tests), speed, stretch, ...)
    src/ui/               frontend: index.html, main.ts, app.css, App.svelte,
                          components/ (library/ = tree, pane, tablature panel;
                          import/ = source page (URL box, or local file/folder path box + Browse +
                          Extract), edit pane, extraction progress, batch progress (BatchProgress);
                          editor/ = EditorPane, StemLane, MixLane, TimeField (also used by the Player);
                          player/ = PlayerPane, SourceSelect, WaveformView (shared with the Editor's Mix lane, props-driven;
                          Player mode `freeMove` edits sections), SectionBpmInput (BPM overlay), BpmSection, SectionsSection,
                          PlaybackSection (+ SpeedControl), LoopsSection (+ TrainingPopover) (row 1: Sections with BPM inside + Loops;
                          row 2: Playback with the Speed group);
                          library/TrackBrowser.svelte = tree column shared by Library and Editor;
                          TrackFields.svelte shared by Library and Import),
                          views/ (one file per main view),
                          lib/ (ipc.ts, views.ts, theme.ts, app-state.svelte.ts,
                          fuzzy.ts, library-tree.ts, track-draft.ts, library-state.svelte.ts,
                          library-order.ts (reorder patches, drop gaps, neighbour after delete),
                          library-rename.ts (band/album rename plans), library-ops.svelte.ts (reorder/delete actions),
                          url-check.ts, import-state.svelte.ts, editor-state.svelte.ts, time-format.ts,
                          player-state.svelte.ts, bar-format.ts, bpm-format.ts, speed-format.ts, waveform-geometry.ts,
                          utils.ts, components/ui/ = shadcn components owned by us),
                          *.test.ts next to the code
    dist/                 generated by `vite build`, gitignored, embedded into the binary
    tests/                cargo integration tests (cli, frontend, acceptance_*, gui_smoke, gui_e2e,
                          gui_library_e2e, gui_import_e2e, gui_editor_e2e, gui_player_e2e, player_realtime); fixtures/library-sample/ = sample
                          repository (v1), library-v2/ = mixed v1/v2 incl. a stem track,
                          import/ = generated audio/video/stem fixtures (make-fixtures.sh),
                          library-editor/ = stem tracks for the Editor (editor/make-fixtures.sh),
                          library-player/ = tracks for the Player incl. an MP3 backing and a player.json
                          with two tempos and loops (player/make-fixtures.sh)
      support/            test-only stand-ins: stub-separator (bash), stub-beat-tracker (Python 3 stdlib),
                          bin/yt-dlp (Python 3 stdlib)
  calliope-lib/           shared library (package calliope-lib, lib calliope_lib): Cargo.toml,
                          src/{lib,stems_api,stems_client,flac_level,process}.rs, tests/; no Tauri
  calliope-stems/         the edge-AI stems server: Cargo.toml, src/*.rs, tests/conformance.rs,
                          separators/audio-separator.sh, trackers/beat-this.sh,
                          deploy/calliope-stems.service, README.md;
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
- Prerequisites: Rust, `webkit2gtk-4.1 gtk3 base-devel alsa-lib clang` (clang = libclang for the time-stretch binding's
  bindgen; the C++ compiler comes with base-devel), Node.js >= 22.12 with npm; run
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
- The workspace `Cargo.toml` builds `claxon`, `flacenc` and `signalsmith-stretch` (its vendored C++ is compiled by the
  crate's build script with the package's opt-level) with `opt-level = 3` even in the dev profile, so debug builds and
  tests decode/encode/stretch audio at a usable speed.
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
  `src/repository_tests.rs`. These are module tests of the library target (they run once, in
  the `lib` test binary).
  Each test also checks that a sibling "outside" folder is unchanged. The committed sample
  repository `tests/fixtures/library-sample/` is always copied to a temp dir first.
- **Tests never touch the user's data**: no test reads or writes `~/.local/share/calliope`,
  `~/.config/app.calliope.gui` or the OS trash. Rust tests take paths as parameters (no env
  lookups), frontend tests fake IPC, and GUI tests use temp XDG dirs and assert that the
  repository root is under `target/`.
- `calliope-gui` is a library (`src/lib.rs`, crate `calliope_gui`, declares all modules) plus a
  thin binary (`src/main.rs`: CLI dispatch, then `gui::run()`). Integration tests in `tests/`
  `use calliope_gui::...` (no `#[path = "../src/..."]` includes, which re-ran every module's unit
  tests in each test file). Modules the tests need are `pub mod`; items they need are `pub`,
  the rest stays `pub(crate)`. Test doubles the integration tests need (`ManualBackend`,
  `ScriptedPicker`) are `#[doc(hidden)] pub` and always compiled; only the env-driven
  `ScriptedPicker::from_env` stays behind `e2e-hooks`. `build.rs` still includes `version.rs`
  by `#[path]` (it cannot depend on the crate).
- CLI behaviour is tested headlessly by running the built binary (`CARGO_BIN_EXE_calliope-gui`)
  with `DISPLAY`/`WAYLAND_DISPLAY` removed.
- Static checks (`tests/frontend.rs`): strict production CSP (no `'unsafe-inline'`/`'unsafe-eval'`/`ws:`), no `devCsp`/`devUrl` in `tauri.conf.json`, `devCsp` in `tauri.dev.conf.json` = production CSP plus only the two allowed tokens, window config, no
  inline code in `src/ui/index.html` or `dist/index.html`, no static `style=` in `.svelte`,
  all assets local (woff2 bundled, no `http(s)://` or `data:` URLs in CSS).
- Native file dialogs in GUI tests: the cargo feature `e2e-hooks` (only `npm run test:gui`
  enables it) plus the env var `CALLIOPE_E2E_DIALOG_ANSWERS=<file>` replace the dialogs with a
  scripted picker (one line per dialog: `<kind> <abs path>|CANCEL`; kinds include
  `import-audio`, `import-video`, `import-directory`). The scripted picker records each request's start folder
  (`ScriptedPicker::starts()`, used through `Arc<ScriptedPicker>` in Rust tests); GUI tests assert the start folder from
  the `calliope: dialog start` log line. Without the env var the
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
  `npm run test:gui` (built with `--features e2e-hooks`; see "Parallel GUI runs" below). Library e2e tests also
  assert on the files on disk. They launch the real binary with temporary
  `XDG_CONFIG_HOME`/`XDG_DATA_HOME`/`XDG_CACHE_HOME` (never the user's real config), assert
  on the `calliope-ui:` stderr lines and on the settings/window-state files, fail on any
  `csp-violation`, and save screenshots to `target/gui-shots/` for visual review. "Offline"
  is a launch under `unshare -rn` (no network namespace; X11 still reachable). Agents never
  open windows on any other display. (The earlier note that agents have no display server is
  outdated.)
- **Parallel GUI runs**: `npm run test:gui` runs `scripts/test-gui.sh`: build once (`npm run
  build`, `cargo build -p calliope-stems`, `cargo test --no-run --features e2e-hooks`), then
  every GUI test binary on its own private X display (Xvfb, or `/usr/lib/Xorg` with the dummy
  driver when Xvfb is not installed; displays `:90` and up, each with its own `i3` because the
  tests float windows with `i3-msg`), with `--ignored --test-threads=1` inside a binary (tests of
  one binary stay serial). The user's `$DISPLAY`, `WAYLAND_DISPLAY` and `XAUTHORITY` are not
  passed on and `HOME` is a folder under `target/gui-e2e/`. Binaries don't share state: each
  test owns `target/gui-e2e/<test>/` (test names are unique across binaries, keep it so),
  screenshot names are unique, `calliope-stems` is only started inside `unshare -rn` (its fixed
  port 8765 is per network namespace, so it also never meets a real server on the host), and the
  tests are built once. Logs: `target/gui-logs/<binary>.log`. `DISPLAY=:1 npm run
  test:gui:serial` runs the same binaries one after the other on an existing display.
- **Screenshot sweeps** (themes x sizes x states) only run with `CALLIOPE_GUI_SHOTS=1`
  (`gui_player_e2e::visual_review`); by default each test takes its normal screenshots only.
- **Beat detection** → the real `calliope-stems` with `--beat-tracker tests/support/stub-beat-tracker` (modes
  `ok` (120 BPM from 0.5 s for whole bars up to half the input, then 90 BPM), `steady`, `few`, `fail`, `slow`, `hang`,
  `bad-output`, via `STUB_TRACKER_MODE` or `<work dir>/stub-tracker-mode`; argv to `$STUB_TRACKER_LOG`) and
  `--beat-timeout-ms`; an "old server" is the binary without `--beat-tracker`. The real adapter is only syntax-checked
  (`bash -n` and `ast.parse` of its embedded Python); no test runs Beat This! or PyTorch. Tempo sections are tested on
  synthetic beat lists (`tempo_detect`), the mix waveform's 0.3 s debounce with injected `Instant`s and a blocking peaks
  hook (`EditorManager::set_peaks_fn`), the detection job against the real server with 50 ms polling.
- **Player tempo-section edits** → pure `tempo_map` tests (operations, refusals, monotonic `ms_at`/`tick_near` around
  placed changes), `player_tests` (grid upload, 500 ms write, stale guard) and `tests/acceptance_player_backend.rs` `ts*`
  (ManualBackend: click onsets at exact frames before/after an edit, source samples continuous across an edit while
  playing, reopen from disk); fixture maps with detected changes are written into the temp copy as `player.json`. The GUI
  test `gui_player_e2e::player_tempo_sections` checks the clicks in the capture WAV. The Player pane logs
  `calliope-ui: player layout overflow=<px> width=<px>` after its size settles, so GUI tests assert the scroll target
  instead of judging scrolled screenshots only.
- Test-only server option `--separator-timeout-ms N` (hidden like `--janitor-interval-ms`, min
  100, wins over `--separator-timeout-min`) lets the acceptance test exercise the separator
  timeout in about a second.
- Rust suites run with `--workspace` (`cargo test --workspace`, `cargo clippy --workspace
  --all-targets -- -D warnings`); `npm test` builds `calliope-stems` first.
- **Edge-AI server** → the real `calliope-stems` binary (`target/debug/calliope-stems`, or
  `CARGO_BIN_EXE_calliope-stems` in its own tests) with `--listen 127.0.0.1:0` (port printed
  on stderr) and `--separator tests/support/stub-separator` (canned stems from
  `tests/fixtures/import/stems/`; modes `ok|fail|slow|hang|bad-output|not-flac|sparse|silent|undecodable|boundary`
  (the canned `stems/` are 16 s tones, 16 s audible; `sparse`: piano all zeroes, other = short loud
  bursts (9.5 s audible, peak -8 dBFS), guitar = eight 2 s phrases (16 s audible), from
  `tests/fixtures/import/stems-quiet/` and `stems-activity/`; `boundary`: guitar 15.0 s, other 14.9 s,
  piano bursts; `silent`: all six all zeroes; `undecodable`: piano is `fLaC` + junk, so the server
  cannot measure it, other is a 1 s quiet tone; `seq:M1,M2,..`: run n (counted in `<work dir>/stub-runs`) uses the n-th
  mode, the last repeats, so one file of a directory batch fails (`seq:ok,fail,ok`) or hangs (`seq:ok,hang`)) via
  `STUB_SEPARATOR_MODE` or `<work dir>/stub-mode`; argv logged to `$STUB_SEPARATOR_LOG`). The
  real adapter is only syntax-checked (`bash -n`); no test runs audio-separator or the model.
  The conformance suite `src/calliope-stems/tests/conformance.rs` checks every endpoint with
  raw HTTP and with the shared client. An "old server" is the real binary with `--no-stem-levels`
  (or the scripted fakes in `tests/acceptance_silent_stems.rs` (pre-peaks) and
  `tests/acceptance_server_levels.rs` (server-peaks era, sends only `stem_peaks`)); "a stem was never downloaded" is
  asserted on the server's stderr request log (`request method=GET path=…/stems/<name>`).
- **YouTube / yt-dlp** → `tests/support/bin/yt-dlp` (fake; refuses any host not ending in
  `.example`; behaviours by URL path: `ok`, `http403`, `offline`, `slow`, `no-total`; resumes
  `.part` files; logs its argv to `$FAKE_YTDLP_LOG`). Rust tests pass its path explicitly;
  GUI tests put `tests/support/bin` first on `PATH`.
- ffmpeg/ffprobe are the real local tools (no network) on self-generated fixtures.
- **Typed local paths**: `local_source` is tested on temp files (FIFO, dangling symlink,
  `chmod 000`, folders, wrong extensions) with an injected home (no test reads the real `~`); the same for the start-folder
  rules and the folder listing (hidden files, subfolders, symlinks, FIFOs, name order);
  `tests/acceptance_existing_track_import.rs` drives typed paths (plain, quoted, `file://`) through
  the real import with the stub server, and also holds the directory-batch and Browse start-folder acceptance tests
  (`ac_*`; an app restart = reopening the `SettingsStore` from the same file); GUI tests type absolute paths under
  `target/` only (`media/album/`, `media/three/`, `media/no-songs/` are created by `import_dirs`).
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
- **Player audio** → the same backends. The player's `Engine` is tested directly (`apply`/`render`) and through
  `ManualBackend`: exact source samples, 3 ms fades at jumps, click onsets at exact frames, count-in timing, loop
  repeats. GUI tests analyse the capture WAV: tone presence per source frequency (330/440/550/660 Hz fixtures) and
  click onsets in the 1.5-2 kHz band. Non-FLAC sources go through the real local `ffmpeg` on a generated MP3 fixture.
  **Speed** (gui-player-speed): the stretcher on synthetic tones (440 Hz ± 2 Hz at 50-200 %, duration ± 1 %, burst
  alignment ± 10 ms); the engine's click-on-beat rule by two renders with the same commands (clicks over a silent source,
  beat bursts without clicks: same timeline, onsets compared); continuity as "no 10 ms window below 50 % of the tone's
  RMS" across speed changes, wraps and training passes; `tests/player_realtime.rs` counts Rust allocations on the render
  thread (must be zero after construction; C++ allocations are covered by code reading). At 100 % the engine stays
  bit-exact. Listening quality is a manual check.
- **Library order and delete** (gui-library-album-order): planners are pure TS (`library-order`, `library-rename`) tested in
  vitest; component tests drive pointer drags with `fireEvent` on `window` and stubbed `getBoundingClientRect`. GUI tests
  use seeded temp repositories (`tests/common` `seed_dirs`: only the given schema-2 tracks), drag with `drag_xy`
  (xdotool mousedown, stepped mousemove, mouseup), use `alt+Up`/`alt+Down`/`Delete` keys, and wait on the `reorder`/`drag`/
  `deleted`/`rename` log lines. A track that cannot be deleted is made by changing its `track.json` after the Library
  loaded (revision conflict); Rust tests also use `chmod 0555` on the track folder (skipped as root, e.g. under
  `unshare -r`). Tagged import sources are made at test time with the local `ffmpeg -c copy -metadata track=…`
  (`tagged_copy`).
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
The owner rejected `symphonia` permanently; no MPL-2.0 decoder is used. (Note 2026-10-11: the rejection came from its
MPL-2.0 licence. Calliope itself is now MPL-2.0, see the licence entry below, so the owner can reconsider; the rule
stands until the owner lifts it.) FLAC is decoded with `claxon`.
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

### 2026-10-09: Silent stems are dropped by the GUI at import; "silent" = sample peak below -50 dBFS   (feature: gui-stem-extraction, increment clear-empty; threshold is an owner decision; the peak criterion is superseded by the 2026-10-10 audible-time entry)
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

### 2026-10-10: The server measures stem peaks; the app skips stems it reports silent   (feature: gui-stem-extraction, increment server-peaks; owner direction; `stem_peaks`/`flac_peak` are replaced by `stem_levels`/`flac_level` in the audible-time entry below, the structure and trust model stay)
Requirement 9: silent stems should not be downloaded. Owner direction: the server only measures and
reports; the keep/drop decision and the -50 dBFS threshold stay in the app; the change is additive and
works across old/new app and server. The integer peak scan moved from the GUI into
`calliope_lib::flac_peak` (`claxon` added to the lib; still no Tauri/GTK/HTTP server there), so server
and app share one implementation, including `limit(bits, dbfs)` and `is_below`. After the separator
succeeds the server decodes every stem in full (no threshold, so no early exit), outside the job lock,
checking cancel/stop between stems, and only then marks the job `done`; a stem that cannot be decoded
gets no entry and the job still succeeds. The job status gains optional `stem_peaks` with the **integer
peak and bit depth** (plus `peak_dbfs` for humans), so the app applies exactly the same integer rule as
its own scan; a float dB value on the wire could round differently at the boundary. "Not measured" is
a missing entry, digital silence is `peak: 0`. The key is omitted (not null) when absent, so
`--no-stem-peaks` reproduces the old JSON byte for byte and doubles as the test stand-in for an old
server. The app still measures every stem it downloads (early exit makes audible stems nearly free, and
the fallback is the same code); only stems the server reports silent are skipped. This supersedes "silent
stems are still downloaded" in the 2026-10-09 entry; the threshold, the all-silent failure and
"undecodable = kept" are unchanged. Alternatives: the server drops stems itself (rejected by the owner:
the decision belongs to the app), sending only `peak_dbfs` (rejected: float boundary drift),
`stems` as objects (rejected: breaks old clients), a new endpoint or API v2 (rejected: unnecessary for an
additive field), trusting server peaks for kept stems without a local check (rejected: no saving worth a
second code path).
Trust model: the app drops stems it never downloads on the server's word. Accepted because the server is
LAN-only, unauthenticated by design, and already supplies the stem audio itself. A server claiming every
stem silent makes the import fail visibly; a stem claimed audible but actually silent is still caught by
the local check. The user sees dropped stems in the dropped list (the server-reported level is in the log).

### 2026-10-10: Empty stems are judged by audible time; the server reports `stem_levels`   (feature: gui-stem-extraction, increment active-time; criterion is an owner decision)
Requirement 9 was rewritten after two real htdemucs_6s imports: empty stems kept separation artifacts
with peaks up to -8.7 dBFS (a 9 s tonal burst in "other"), while real parts were audible for 200-362 s
in total but in runs as short as 15 s. A stem is now empty when it is audible for less than 15 s **in
total**. The measure is `calliope_lib::flac_level`: back-to-back windows of `sample_rate/10` frames
(always counted as 100 ms; a trailing partial window is ignored), per channel the integer sum of
squares, the loudest channel decides, and a window is audible when `S > floor(n·4^(bits-1)·10^(T/10))`
with T = `AUDIBLE_LEVEL_DBFS` = -40. For levels that are multiples of 10 dB this limit is exact integer
arithmetic, so server and app agree bit for bit. `audible_ms` is an integer (windows × 100), so 14.9 s
is empty and 15.0 s kept. The level and window are lib constants (what is measured); the 15 s decision
is `stem_audio::EMPTY_STEM_MIN_AUDIBLE_MS` in the app only. The server scans fully (no threshold) and
reports `stem_levels: [{name, audible_ms, level_dbfs, window_ms, peak_dbfs}]`; the app uses an entry
only when level and window equal its constants, so a server built from another version can only cause
a fallback to downloading. The app's local check stops at 15 s audible (cheap for real parts; a full
scan only for empty stems, about 0.2 s per 4-minute stem). `stem_peaks`, `flac_peak` and
`--no-stem-peaks` were removed rather than kept: never released, and a peak can no longer decide
anything; the new key name means a deployed server-peaks build is simply treated as a server without
audible time (everything downloaded and checked locally, as the spec asks). `DroppedStem` carries
`audible_ms`; the UI says "empty" instead of "silent". `calliope-stems --measure FILE...` prints the
same numbers for any FLAC file, so the owner can check existing tracks. The canned test stems became
16 s long (1 s stems would all be empty) and `stems-activity/` fixtures cover bursts, phrases and the
14.9/15.0 s boundary. Alternatives: longest continuous run (rejected: a sung part's longest run was
14.9 s), peak or whole-stem RMS/LUFS (rejected: artifacts are loud, sparse parts are quiet on average),
the app sending the level per job as a query parameter (rejected: more protocol for no gain while both
binaries come from one repository; the echoed level already protects against skew), a per-level
histogram in the report (rejected as premature; the roadmap item `stems-empty-detection` may change the
measure itself), using a server-peaks build's `stem_peaks` as a shortcut (peak < -40 dBFS implies empty;
rejected: extra code path for a never-released server), keeping a trailing partial window (rejected:
needs its own limit and changes at most 99 ms).

### 2026-10-10: Local-file import takes a typed path; Browse fills the box   (feature: gui-existing-track-import)
The spec asks for a text box for the path next to Browse for local audio and video files. The
import itself is unchanged: local files go through stem extraction exactly like URLs (owner
clarification); there is no as-is import. Choice: Browse (`pick_import_file`) only fills the box
with the full path, and Extract/Enter starts `import_path(kind, path)`. The box always shows what
will be imported and can be corrected, the usual "path field + Browse" pattern. This is a narrow
exception to "the frontend never sends paths" (2026-10-05): one read-only command, validated in
pure `local_source` (absolute after `~/`/quotes/`file://`, regular file, readable, audio/video
extension; no canonicalisation, the path reaches ffmpeg as `file:<abs>` like a dialog path).
Risk accepted: a compromised webview could at most have a media file's audio uploaded to the
owner's own edge-AI server, which Browse already allows. Every other command keeps tokens.
Check-then-open window: a file swapped for a FIFO after the check could block ffprobe; cancel still works.
`import_file` (dialog then immediate start) was removed rather than kept beside the new
commands, so there is one way to start a local import. Alternatives: Browse keeps starting at once
and the box is a second way in (rejected: two behaviours on one row, and the box would not show
the browsed file), and a token for browsed files plus a path for typed ones (rejected: two code
paths for the same thing, with no security gain once typed paths are accepted). A video's first
audio stream is used (`-map 0:a:0`, unchanged).

### 2026-10-11: Player settings live in `player.json` next to `track.json`   (feature: gui-play-backing-track; owner-confirmed)
The Player saves every adjustment (source, tempo map, speed, per-section Click / Count In flags, loops, playback position) with no Save button. Writing
`track.json` that often would change its revision on each loop edit, so a Library edit in progress would hit a
save conflict, and the Library would rescan "modified" tracks. Choice: a separate `player.json` in the track folder,
debounced (500 ms) atomic writes, lenient read, unknown fields kept, a newer schema never overwritten, the name
reserved like `track.json`, copied by Export, moved with the folder by Delete, ignored by scans. One state per track,
shared by all sources, because stems and backings are sample-aligned (owner-confirmed). Changing the source stops
playback and keeps the position (owner). Alternatives: a `player` object in
`track.json` (rejected for the conflicts), a per-source state (rejected: duplicated tempo maps and loops that drift
apart), a hidden dot file (rejected: not "easy to hack with"). The tempo map may move into `track.json` when
`gui-manipulate-backing-track` makes tempo part of the track metadata.

### 2026-10-11: The Player is its own engine; its playback state lives in the audio callback   (feature: gui-play-backing-track)
Count-ins, loop repeats and metronome clicks must happen at an exact sample, without waiting for the 20 ms ticker,
so the Editor's "atomics + manager-side transport" model is not enough. The Player's `Engine` (a pure struct with
`apply(cmd)` and `render(out)`) is owned by the callback and receives commands through a bounded
`std::sync::mpsc::sync_channel` (array flavour: no allocation per message); replaced `Arc`s go back through a second
channel to be freed off the audio thread; status leaves through atomics. The manager keeps the resting state while
no output is open and builds the engine from it on open. The Editor engine is unchanged; both share the output
backend and only one sounds at a time. Alternatives: extending `EditorManager` (rejected: the Editor mixes many stems
with solo, the Player plays one source with timing features, and a merged engine would be harder to test), a
lock-free ring-buffer crate such as `rtrb` (not needed; std's bounded channel suffices), a mutex shared with the
callback (rejected: priority inversion on the audio thread).

### 2026-10-11: Tempo map with a 1/64-note grid; loops stored as ticks or ms by mode   (feature: gui-play-backing-track; 4/4 owner-confirmed, time signatures come with the tablatures)
A tempo map is a list of segments `{start_ms, bpm}`; none = BPM 0 = timestamp mode. The first segment's start is
bar 1, beat 1 (typing a BPM while paused on the first downbeat sets both tempo and grid); its tempo extends
backwards. A tick is a 1/64 note (16 per quarter-note beat); `beats_per_bar` is stored (default 4). Later tempo
changes snap to the nearest tick of the current grid, so tick bases are integers and conversions are exact. Loop
boundaries are `{tick}` in BPM mode and `{ms}` in timestamp mode (the spec's rule) and are re-snapped, keeping their
time, whenever the tempo map changes, so a loop stays on the audio the user marked. Speed is one factor per track;
the BPM shown is `bpm_at(position) * speed`. At BPM 0 there is no speed change (`-`/`+`/`Reset` disabled, owner).
(**The speed part is superseded** by the 2026-10-18 gui-player-speed entries: a percentage independent of the tempo map,
also without one; the BPM field shows the song BPM and a separate readout the played BPM.)
A loop is created with `+` at the position and **no end** (owner): it is "incomplete" (stored `end: null`, never
repeats, Play/Loop On/Count-In disabled, start mark only) until `<` sets an end after the start; `>` at or after the end
moves the start and clears the end again, so a loop is never silently reversed.

### 2026-10-11: Metronome and count-in are rendered in the playback stream   (feature: gui-play-backing-track)
The click is our own synthesis (decaying 2 kHz / 1.5 kHz sine, -6 dBFS) mixed by the same callback that plays the
track, after any speed change, so ticks are sample-aligned with the music whatever the device latency. This settles
the Editor-era note "device latency is not compensated; the Player must revisit": for audio-to-audio sync no
compensation is needed. Displayed positions still lag by the device latency (tens of ms); the tablature cursor and
MIDI clock features must compensate when they need tighter visual/MIDI sync.

### 2026-10-11: Non-FLAC sources are decoded by the external ffmpeg into memory   (feature: gui-play-backing-track)
Plain backing tracks may have MP3/OGG/... audio, and `symphonia` is rejected. The Player runs `ffmpeg` (already a
runtime prerequisite) with argv only, `-protocol_whitelist file,pipe`, `file:<abs>` input, `-f s16le -ac 2 -ar 44100`
to a pipe and reads the PCM into memory; no cache file is written. FLAC sources keep using `claxon`.

### 2026-10-11: Calliope is licensed under MPL-2.0 (was Apache-2.0)   (owner decision; applied by gui-play-backing-track plan task 18)
The owner switches all three crates and the npm project to the Mozilla Public License 2.0. `LICENSE` holds the exact
official MPL-2.0 text; `package.license` in the workspace `Cargo.toml` (inherited by every crate) and `license` in
`package.json` say `MPL-2.0`; the README carries the MPL Exhibit A notice (no per-file headers, which MPL allows when
the notice is where recipients will look). The third-party notices generator no longer takes the Apache-2.0 standard
text from `LICENSE` but from `licence-templates/Apache-2.0.txt`. Apache-2.0, MIT, ISC, BSD and OFL dependencies stay
compatible (MPL is file-level copyleft and covers only Calliope's own files). Consequences: MPL-2.0 dependencies
(e.g. Bungee for time-stretch) no longer clash with the project licence, and the "no symphonia" rule, which came from
symphonia's MPL-2.0 licence, may be reconsidered by the owner (not lifted). LGPL code linked statically would still
bring relinking obligations for distributed binaries.

### 2026-10-11: Beat detection = Beat This! on archserver behind additive `calliope-stems` endpoints; the app derives the tempo sections   (feature: stems-beat-autodetect; owner decision, training-data note accepted)
The spec delegates AI processing to `calliope-stems`. Beat This! 1.1.0 (CPJKU) finds beats and downbeats without assuming
one tempo; its code and published weights are MIT (some training data is not, noted in `docs/licences.md`). Choice: a
configurable **beat-tracker command** with a small contract (`<tracker> <input.flac> <out_dir>` → `beats.json`), exactly
like the separator, so the model can change without touching the server and tests use a stub; additive endpoints
(`POST /v1/beats`, `GET /v1/jobs/<id>/beats`, `tasks` in health, `kind` in a beats job's status) in the one job queue (one GPU);
the real adapter runs in its own uv venv `~/edge-ai/beats/.venv` (an upgrade can't break Demucs) with a pre-downloaded
checkpoint, CUDA with CPU fallback. The server only reports beats (validated by the shared `check_beats`); the app turns
them into sections (`tempo_detect`), the same "server measures, app decides" split as `stem_levels`. Alternatives: a local
non-AI detector in Rust (weaker on real music and tempo changes), madmom (non-commercial models), aubio/Essentia (GPL/AGPL),
librosa (one tempo only), a new API version (unnecessary for additive routes).

### 2026-10-11: One tempo map per track, in `player.json`; the Editor's writes close the Player's session   (feature: stems-beat-autodetect; owner decisions 1 and 8a)
The detected tempo serves the metronome and count-in, which read `player.json`, and all sources of a track are
sample-aligned, so the Editor writes the same `tempo`. Choice (owner): no live hand-over. `TempoStore::replace` runs under
the `PlayerHost` lock, flushes and closes a Player session holding the track, then rewrites the file from its flushed
content (only `tempo` and the loops' ticks re-expressed at the same times change; `speed` was reset too until
gui-player-speed, which keeps it). The Player reopens the track
when next shown; `finish_open` re-reads a file whose revision changed since its off-lock load. So the Editor never loses
the Player's loops or position, and the Player can never write the old tempo back. Detection replaces the whole map
without a dialog; the raw beats are not stored. Alternatives: pushing the new map into the open session (rejected by the
owner), `track.json` (Player change, revision churn on every typed BPM), a copy per backing variant (goes stale). The
2026-10-11 `player.json` entry's note still holds: the map may move into `track.json` with `gui-manipulate-backing-track`.

### 2026-10-11: Detected tempo changes: 1/64-beat positions in ms with a stored detected position; `player.json` schema 2   (feature: stems-beat-autodetect; owner decisions 4-6)
The owner set the move step to 1/64 of a beat (4 beats = 256 steps) and the limit to ±4 beats from the detected position
in total, and allowed sections of one bar. The tick grid (1/16 beat) holds the Player's loops, so it stays. Choice: change
positions stay in `start_ms` (as before) and a detected change also stores `detected_ms`; it is placed and moved in
`fine_ms = 60000/(bpm*64)` steps of its left section's tempo, always measured from `detected_ms` (the frontend sends the
absolute offset). A detected change starts its section on a beat: its tick base rounds to the nearest whole beat and the
left section's last beat is stretched to reach it, keeping `ms_at`/`tick_near` monotonic and inverse. Typed changes, loop
snapping and existing maps are unchanged. `player.json` becomes schema 2 so older builds can't drop `detected_ms` or
re-round bases. Sections come from an optimal segmentation (least-squares line per section, PELT, penalty from the measured
beat jitter), at least 4 beats long, merged when within 4 %; isolated missed/double beats are repaired first. A 1-bar
section is found when it differs by > 4 % and the jitter is ≤ about 10 ms. Alternatives: raising `TICKS_PER_BEAT` to 64
(changes how every loop snaps, needs a loop migration), a second fine tick grid (two grids to keep consistent), the
earlier median rule (cannot see a 1-bar section), per-gesture limits (rejected by the owner).

### 2026-10-11: The Editor's mix waveform is computed in Rust, debounced 0.3 s, on a worker thread   (feature: stems-beat-autodetect)
The frontend never keeps time, so the 0.3 s restartable wait is a pure `Debounce` in the editor session driven by the
20 ms ticker with injected `Instant`s. The peaks are the playback mixer's own maths over the in-memory i16 stems (what Play
and Save produce), on a worker thread that never takes the manager lock, with a generation number and a cancel flag, so
IPC, playback and the audio callback never wait. 2048 buckets, same bucket rule as the Player's peaks. The Player's canvas
component became props-driven and is shared; the Player shows the tempo sections read-only (owner 8b, 8c; made editable by the 2026-10-14 gui-player-tempo-sections entries). Alternatives: summing per-stem peak envelopes
(wrong: max of a sum is not a sum of maxes), recomputing in the webview (needs the PCM in JS), a frontend timer (keeps time
in JS).

### 2026-10-14: The Player edits tempo sections live in its own session   (feature: gui-player-tempo-sections; plan §2.2, owner Q1: (a), 2026-10-14)
The user corrects sections while hearing the metronome against the song, so an edit must take effect at once and must not
stop playback. Choice: the five section edits are new kinds of the existing `player_tempo` action and run in
`PlayerManager::edit_sections`, the same live path as a typed BPM (`apply_tempo_change` → `Cmd::SetGrid` → `sync_loop` →
debounced write); the Editor's `TempoStore::replace` (flush + close the Player session) stays the Editor's path only.
Index-based actions carry the section's current `start_ms` and are refused when it no longer matches, so quick repeated
clicks never hit the wrong change. The actions are reached through a "Sections" group in the control bar (acting on the
selected section, else the one at the playhead), clicks on the waveform's BPM labels (an input drawn over the label) and
keys on the focused waveform; Space, b, e and L keep their Player meaning there. Alternatives: releasing the session like
the Editor does (stops playback, contradicts the spec), a context menu (hidden, mouse-only), new commands (the tagged
action keeps the command list short and needs no ACL change).

### 2026-10-14: Player-added tempo changes start on a beat (`on_beat`), `player.json` schema 3   (feature: gui-player-tempo-sections; plan §2.3-2.4, owner Q2-Q4: recommended answers, 2026-10-14)
A change placed by the user must make the metronome's first click of the new section land on it. The typed-BPM tick rule
continues the old beat count, so the first click could land up to 15/16 beat later. Choice: a per-segment `on_beat` flag
that applies the detected-change rule (beat-rounded base, stretched last beat) without a detected position; such changes
sit on the left section's 1/64-beat grid counted from its start and move freely between neighbours (every section ≥ 1 beat
of its own tempo). Detected changes keep the ±256-step limit from `detected_ms`, widened to their current offset so a later
change of the left BPM never forces a jump. A typed change becomes `on_beat` when first moved; the BPM field itself is
unchanged. Section BPM edits never merge equal neighbours (a same-tempo change still re-aligns the beat). Schema 3
because the schema-2 parser drops unknown segment keys and would re-round the bars. Alternatives: the tick rule for added
changes (no schema bump, clicks off the bar), making every change beat-aligned (changes the meaning of existing typed
maps and their loop ticks), storing placed changes as detected ones with a "free" key (confusing data, old builds would
silently limit them).

### 2026-10-17: Per-section metronome and count-in are a click list built by the manager; Metronome On/Off is the master switch; `player.json` schema 4   (feature: gui-player-tempo-sections phase 2; plan §10.2-10.6, recommended answers to phase-2 Q1-Q9, confirmed by the owner 2026-10-17; the master switch and the global Count In are superseded by the 2026-10-18 entry)
Each tempo section can be named, silenced and given a count-in that plays while the song runs into it. Choice: the three
settings live in the section's segment (`SectionMeta`, flattened, defaults not written) so they move with their change
through every edit; the manager turns the map into the list of click frames (`metronome::section_clicks`: beats of
sections whose metronome is on, then for each section with count-in three clicks at its tempo ending one beat before
its first beat, replacing the clicks in that span, skipped when a repeating loop ends there) and sends it with the
existing `Cmd::SetGrid`. The engine and its real-time rules are unchanged, and the engine's existing metronome flag
(top-level `metronome.on`) becomes the master switch. `Count In` stays the pre-roll before Play and loop Play. Schema 4
because the schema-3 parser drops unknown segment keys. Alternatives: per-beat flags evaluated in the audio callback
(more real-time code, harder to test), settings in a separate list keyed by section index (goes out of step on every
Add/Remove), dropping the master switch (no quick way to silence everything while playing).

### 2026-10-18: No global metronome: per-section Click and Count In only, Play counts in per section; `player.json` schema 5   (feature: gui-player-tempo-sections phase 3; plan §18.3-18.6; spec requirement 14; confirmed by the owner 2026-10-18, phase-3 Q1-Q8)
The owner removed Metronome On/Off and the global Count In: what clicks is decided per section only. Choice: the engine's
metronome flag now only gates "has a tempo" and `section_clicks` alone decides the clicks; Play's pre-roll is taken from
the Count In of the section under the playhead (allowed on the first section, where it means only that pre-roll); loop
Play uses only the loop's Count-In; the `player_metronome` command and the top-level `metronome` object go. Schema 5,
because a schema-4 build would read a missing master as Off and drop the first section's count-in. Old files are migrated
so that they sound the same (master Off → every section silent without count-ins; master Count In → the first section's
Count In); Play mid-song counts in through the Count In of the section under the playhead, and only the old global
pre-roll before loop Play is not carried (loop Play uses the loop's own Count-In). Autodetect copies the old first section's Click to every new section and its Count In to the new first
section, so a re-detection keeps a quiet track quiet. A new loop ("+") ends at the next tempo section's start tick, or
at the track end. Alternatives: keep the top-level object as a hidden master (contradicts the spec), migrate the master
Count In to every section (adds count-ins while playing through), leave old files unmigrated (tracks with Metronome Off
would start clicking). Layout (owner, Q8): under the waveform two full-width rows, Sections (with the BPM group) beside
Loops, then Playback at the bottom; the pane logs `player layout overflow=… loops_beside=…` so GUI tests check that the
pane scrolls at most 14 px at 1280x800 with the tree folded and that Loops sits beside Sections there; narrower
windows wrap Loops below Sections, never sideways.

### 2026-10-18: Time-stretch with Signalsmith Stretch through `signalsmith-stretch` (MIT), behind `stretch.rs`   (feature: gui-player-speed; plan §2.2)
The Player needs a real-time speed change from 25 % to 200 % with the pitch kept, under a permissive licence (spec), in the
existing callback, no symphonia. Choice: Signalsmith Stretch 1.3.1 (MIT C++, vendored in the MIT binding
`signalsmith-stretch` 0.1.3), wrapped by our `Stretcher` so the binding can be swapped or vendored in one file. Measured on the
laptop: 0.8 % of one core (release, stereo 44.1 kHz) at any speed, 60 + 60 ms latency, a 440 Hz tone stays 440 Hz (440.9 at
25 %). Costs: `clang`/libclang at build time (bindgen) and a C++ compiler; dev builds compile the C++ at opt-level 3; the two
C++ licence texts sit in sub-folders, so the notices generator lists them explicitly. Alternatives: Bungee (MPL-2.0, not
permissive as the spec asks, CMake), SoundTouch (LGPL) and Rubber Band (GPL) excluded, pure-Rust `timestretch` (unproven on
guitar) and `wsola` / an own WSOLA (audible doubling on chords and full mixes), an own phase vocoder (weeks of tuning). The
earlier plan's `Stretcher` trait is dropped: one implementation, one module.

### 2026-10-18: The engine plays stretched audio on a song-time timeline; clicks are placed after the stretch   (feature: gui-player-speed; plan §2.4)
Clicks, count-ins, wraps, the playhead and training passes must stay on the music although the stretcher delays it.
Choice: at exactly 100 % (no training run) the engine keeps today's bit-exact bypass path; otherwise it feeds the stretcher
in sub-blocks (≤ 256 frames, cut at loop and track ends) and records, in a fixed ring, which song frame each output frame
plays (fed position − input latency, heard output latency later). The heard song position drives `Shared.position`, the
click list (clicks are synthesised after the stretch, so they are never stretched and stay on the beats), wraps and ends.
Every discontinuity (Play, Jump, Resume, PlayLoop, wrap with count-in) aligns the stretcher: seek with a pre-roll ending at
target + input latency, then process and discard the output latency (the library's own `exact()` method; verified with
bursts at 50/85/150 %, within one 10 ms window). Speed changes while playing only change the feed ratio; entering stretch
mode from bypass crossfades over 3 ms. Loop wraps without count-in feed straight on (the stretcher overlaps the ends);
ends with count-in, loops played once and the track end let the tail be heard before acting. Alternatives: always
stretching (loses bit-exactness and every exact test), placing clicks by the fed position (they would run ~120 ms early),
re-aligning at every wrap (an audible seam each pass), stretching the clicks with the music (smeared clicks).

### 2026-10-18: Speed is a whole percentage per track, independent of the tempo map; `player.json` schema 6   (feature: gui-player-speed; plan §2.3, §2.7; replaces gui-play-backing-track requirement 14's BPM steps)
The spec asks for 25..200 % in 5 % steps, typed values and Reset, also without a tempo map, saved per track. Choice: the
track speed is a whole percentage (`speed.rs` rules; `-`/`+` snap to the 5 % grid), stored as the existing `speed` factor
(now 0.25..2.0), so old files (always 1.0) open at 100 %. Typing a BPM and Autodetect no longer reset it (it no longer
derives from a BPM). The Sections BPM box shows the song BPM; the Playback pane shows `bpm × speed` read-only. Schema 6,
because a schema-5 build would clamp and rewrite the speed. Alternatives: a new `speed_pct` key (two keys for one value
across versions), keeping schema 5 (old builds would turn 25 % into 50 %).

### 2026-10-18: Speed training runs in the engine; absolute percentages; the run ends with the loop's Play   (feature: gui-player-speed; plan §2.6, §2.11; owner decisions 2026-10-18)
Each new speed must start exactly at the loop start without a gap, which the 20 ms ticker cannot guarantee. Choice: the
loop's settings travel in `LoopSpec` as a plan; on the loop's own Play the engine runs `pass_speed(pass)` and switches the
fed speed at each wrap on the feed side, while the timeline publishes the pass and speed when they are heard (atomics, so
the manager shows "70 % · 2/4"). Training percentages are absolute (not scaled by the track speed). A run always uses
stretch mode, ends on Pause, loop Stop, leaving the loop, another loop or Training/Loop On off, and then returns to the
track speed; the speed control is disabled while it runs. Training needs repeats, so Training on switches Loop On on.
The settings are edited in a non-modal popover from a "Training…" button in the Loops pane (owner, 2026-10-18; Escape,
click outside or Close close it, keyboard reachable), so the Loops pane keeps three lines.
Alternatives: the manager sending `SetSpeed` on each wrap (late by up to a tick), relative percentages (what is heard
differs from what is shown), training by any playback that wraps the loop (surprising speed changes during normal play).

### 2026-10-19: Browse starts in the typed path's folder, else the last import folder, else home; one shared `last_import_dir` setting   (feature: gui-import-local-batch; plan §2.3; spec Q2, Q3: proposed defaults)
Browse used to open the dialog in home every time. The starting folder is now chosen in Rust by the pure
`local_source::browse_start(typed, last, home)`: the box text goes through the same `normalise` as Extract, then an existing
folder gives itself and any other existing entry its parent; invalid text falls through silently. Next comes
`settings.last_import_dir` if it is still a folder (otherwise it is cleared), then home. `pick_import_file` takes the box text
(`typed`) for this only. The folder is remembered when a local import is *accepted* (the job started): a file's folder, or the
imported directory itself (owner answer to open question 1 of the plan). One field is shared by audio, video
and directory Browse (spec Q2). The dialog only opens the folder and never preselects the file (spec Q3). Alternatives:
keeping the start folder in the frontend (rejected: the webview would have to send and receive paths for this, and
`settings.json` is Rust's), and one last folder per kind (rejected by the spec's proposed default: three settings for one
habit). Tests read the start folder from the scripted picker's recorded requests and from the `calliope: dialog start` log line.

### 2026-10-19: A directory import is one batch job in the import slot; each file reuses the single-file steps without the edit pane   (feature: gui-import-local-batch; plan §2.4-2.5; spec Q1: proposed default)
"Several imports at the same time" is out of scope, and the GPU does one song at a time, so the batch is **one** job
(`SourceKind::Directory`) in the existing single job slot. It is active until it ends (blocking other imports and
root changes) and shares one `Cancel`. Its thread runs, for each audio file directly inside the folder (`list_audio_files`:
`AUDIO_EXTENSIONS`, case-insensitive, no subfolders, no hidden files, case-insensitive name order), the
same `prepare_file` + `extract_cleaned` code as a single Local Audio File import, in its own `import-tmp/file-<uuid>/`.
The tags give the metadata (the file name is the title fallback), with **no edit pane** (spec Q1), so the batch runs
unattended. A file's failure becomes a `BatchItem` with `result: failed` and the batch goes on. Cancel stops the current
file through the existing paths (killed tools, deleted server job, abandoned staging) and skips the rest, and committed
tracks stay. The snapshot gains `batch {dir, files, current, items}` (omitted for single jobs, whose JSON is
unchanged), and the events gain `batch-file`, `batch-item` and `finished`. The single-import `ready`/`saved`/`failed` events
are never sent in a batch. `import_directory(path)` is the second command that takes a path (read-only, like
`import_path`). Alternatives: a frontend-driven loop over `import_path` + `start_stem_extraction` (rejected: the frontend
would orchestrate a long job, a webview reload would stop the batch, and the files would have to be listed to the webview),
and a queue of separate jobs (rejected: two job concepts, and "one import at a time" would need new rules). Tests use the
stub separator's new `seq:` mode to fail or hang one file of a batch.

### 2026-10-20: Album and track order live in each `track.json` (`position`, `album_position`), additive, no schema bump   (feature: gui-library-album-order; plan §2.2)
The tree must show tracks in the publisher's order and a band's albums in chronological order. Choice: two optional
integers per track, `position` (place in the album) and `album_position` (the album's place in the band, copied on every
track of the album; the tree takes the minimum). This keeps the repository "plain folders + one `track.json` per track":
no index file to get out of sync, Export carries the order, and hand edits stay possible. Alternatives: one
repository-level order file keyed by band/album names (rejected: it breaks on every rename, needs its own concurrency and
leaves stale entries after deletes), and a schema bump to 3 (rejected: schema-2 builds already preserve unknown keys and
would then treat every reordered track as read-only). Consequences: an album reorder rewrites every track of the band
(small files, revision-checked); reading is lenient (a bad value = unset), writing strict (1..9999); an album whose tracks
carrying the value were all deleted loses its place until the next reorder.

### 2026-10-20: Group changes are planned by the frontend and applied by Rust as revision-checked batches   (feature: gui-library-album-order; plan §2.4-2.5, §2.8)
Reorders, band/album renames (with merges) and album deletes touch many tracks. The frontend already builds the tree the
user acts on, so it computes the exact per-track changes (`library-order.ts`, `library-rename.ts`) and sends them with each
track's revision. Rust stays a dumb, safe applier: `update_tracks` (and `save_track`'s `group`) checks every track first and
refuses the whole batch on any conflict, then writes only changed files, reporting write failures per track;
`delete_tracks` deletes each track independently and reports failures with their reason (the spec wants the others deleted).
Alternatives: grouping in Rust (rejected: it would duplicate the tree's case- and accent-insensitive grouping without a
Unicode-normalisation crate and could disagree with what the user sees) and a frontend loop over `save_track` (rejected: no
upfront conflict check, many round trips). Consequence: a batch is checked-then-written, not atomic as a whole; after a
partial failure the frontend rescans and shows the true state.

### 2026-10-20: Tree drag and drop uses pointer events, not HTML5 drag and drop   (feature: gui-library-album-order; plan §2.7, open question 6)
HTML5 drag and drop is unreliable in WebKitGTK (drop events missing unless `setData` is called, WebKit bug 265857, also
seen in Tauri), shares GTK machinery with Tauri's file-drop handler, and synthetic xdotool drags through GTK drag and drop
are flaky. The tree implements its own drag with pointer events (5 px threshold, window-level move/up listeners, Escape
cancels, drop gap from the sibling rows' rects via the pure `gapAt`, an inset line as the marker, edge auto-scroll), as the
Player waveform already does. Consequences: no OS drag image (the dragged row is dimmed in place); it works the same with
mouse and touchpad, in jsdom and under xdotool; no Tauri config change. Reordering and deleting exist only in the Library's
tree (`TrackBrowser editable`); the Editor and Player show the same order read-only.
