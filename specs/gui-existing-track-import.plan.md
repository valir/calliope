# Plan: Import track from the local file system (path box next to Browse)

Spec: `specs/gui-existing-track-import.md`. Base: `09d92c2` on `spec/gui-existing-track-import`.

## 1. Summary

The local audio and local video import is already built and merged by `gui-stem-extraction`.
In that import, Browse opens the native dialog and the import starts at once. Requirement 3
already works too: ffmpeg takes the first audio stream of the video, converts it to FLAC, and
the rest is the audio path (metadata, edit pane, stem extraction, staged save). The owner
confirmed that a local file goes through stem extraction exactly like a URL, so **there is no
as-is (no-stems) import in this feature**.

The gap against the spec is the **editable path text box next to Browse** (requirements 1-2):
- the user can type or paste a path (`/…`, `~/…`, a quoted path or a `file://` URI), and Browse
  fills the box;
- Extract (or Enter) starts the existing local-file import from the path in the box;
- a path that is missing, a folder, not a regular file, unreadable or the wrong type is refused
  at once, with a clear accent-colour message under the box, and no job starts.

This needs a narrow, documented exception to the rule "the frontend never sends file-system
paths": one command takes a typed path, and it only ever **reads** that path. Everything after
"we have an absolute path to a readable media file" is reused unchanged: `ImportState::start_file`,
`media`, the import job, the edit pane, extraction and saving. The new tests prove both
acceptance criteria through the typed path, end to end.

## 2. Design

### 2.1 What changes and what doesn't

| Part | Change |
|---|---|
| `import_job.rs`, `media.rs`, `import_tmp.rs`, repository, server, protocol | **none** (`start_file(root, kind, abs_path, sink)` is the entry point, as today) |
| `src/local_source.rs` (new, pure, std + `url`) | typed-path normalisation and checks; the audio/video extension lists move here from `picker.rs` (re-exported there, so other code keeps compiling) |
| `src/ipc.rs`, `src/gui.rs`, `build.rs`, `capabilities/main.json` | `import_file(kind, events)` is **replaced** by `pick_import_file(kind)` and `import_path(kind, path, events)` |
| `src/ui/lib/ipc.ts` | wrappers `pickImportFile`, `importPath` (and `importFile` is removed) |
| `src/ui/lib/import-state.svelte.ts`, `components/import/SourcePage.svelte` | the path row (box + Browse + Extract) and its state |
| tests | Rust unit tests, a new headless acceptance file, vitest, GUI e2e (helpers updated, new tests) |

### 2.2 UI: the path row

When "Local Audio File" or "Local Video File" is selected, the source page shows one row, in the
same layout as the URL row:

```
[ /home/me/Music/song.mp3                    ] [Browse] [Extract]
<accent-colour error text, role=alert, id=import-error>
```

- **Input**: `id="import-path"`, `aria-label="file path"`, `type="text"`, `autocomplete="off"`,
  `spellcheck={false}`. The placeholder is `/path/to/song.mp3` (audio) or `/path/to/video.mp4`
  (video). The input is bound to `imp.path` and uses `aria-describedby="import-error"` when there
  is an error. Pressing Enter does the same as Extract.
- **Browse** (`variant="outline"`): opens the native dialog through `pick_import_file(kind)`. On
  a pick it sets `imp.path` to the returned full path, clears `imp.error`, logs
  `path-picked kind=<audio|video>` and **moves the focus to Extract**, so Enter starts the
  import. Cancelling the dialog changes nothing. Browse does **not** start the import any more:
  the box always shows what will be imported, and the user can check or correct it. This is the
  usual "path field + Browse" pattern.
- **Extract** (primary): if the trimmed box is empty, it shows `Enter the path of a file or
  choose one with Browse` with no IPC call. Otherwise it calls `import_path(kind, path, events)`
  and adopts the snapshot exactly like today's `browse()` (the job goes `preparing` → `ready` →
  edit pane). A rejected path returns `Err(message)`, which `showError` displays. The box keeps
  its text, so the user can fix it.
- All three are disabled while `locked` (the existing rule: starting, a job exists, or a prompt is
  open).
- Switching between audio and video clears `imp.path` and `imp.error`, because a path for one kind
  is useless for the other. "Import another track", `resetImport` and `importAnother` clear
  `imp.path` too.
- The edit pane, the progress page and the done page are unchanged. The source label stays the
  bare file name.

### 2.3 Rust: `src/local_source.rs` (pure)

```rust
pub const AUDIO_EXTENSIONS: &[&str] = …;   // moved verbatim from picker.rs
pub const VIDEO_EXTENSIONS: &[&str] = …;
pub fn is_audio_name(name: &str) -> bool;  // moved verbatim
pub fn is_video_name(name: &str) -> bool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind { Audio, Video }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathProblem { pub reason: &'static str, pub message: String }

pub const MAX_PATH_BYTES: usize = 4096;

/// Syntax only (touches no file): text the user typed → absolute path.
pub fn normalise(input: &str, home: Option<&Path>) -> Result<PathBuf, PathProblem>;

/// normalise + file-system checks. Only reads metadata and opens the file for reading
/// (then closes it). Never creates, writes or follows anything else.
pub fn resolve(input: &str, kind: LocalKind, home: Option<&Path>) -> Result<PathBuf, PathProblem>;
```

`normalise` works in this order:
1. Trim surrounding whitespace. Empty → `empty`.
2. Strip **one** pair of matching surrounding quotes (`'…'` or `"…"`), then trim again (this is
   what pasting from a file manager or a terminal often gives).
3. Longer than `MAX_PATH_BYTES`, or any control character (including NUL, CR, LF, TAB) →
   `invalid`.
4. Starts with `file:` → `url::Url::parse` then `to_file_path()` (percent-decoded; only empty
   or `localhost` hosts). Failure → `invalid`.
5. `~` alone or `~/rest` → `home.join(rest)`. If there is no home, that is `relative`. `~user/…`
   is `relative` (not supported).
6. The result must be absolute (`Path::is_absolute`). Otherwise → `relative`.
7. No canonicalisation and no `..` resolution: the path goes to ffmpeg as `file:<abs>` exactly as
   the dialog path does today.

`resolve` then checks, in this order (`<p>` = `path.display()` of the normalised path, `<n>` =
its file name):

| reason | check | message (exact) |
|---|---|---|
| `empty` | step 1 | `Enter the path of a file or choose one with Browse` |
| `invalid` | steps 3-4 | `The path is not valid` |
| `relative` | steps 5-6 | `Enter the full path, starting with / or ~/` |
| `missing` | `fs::metadata` NotFound (also a dangling symlink) | `No file found at <p>` |
| `folder` | `metadata.is_dir()` | `<p> is a folder, not a file` |
| `not-file` | not `is_file()` (FIFO, socket, device) | `<p> is not a regular file` |
| `unreadable` | other metadata error, or `File::open` fails | `Cannot read <p> (permission denied)` for PermissionDenied, else `Cannot read <p>` |
| `extension` | no file name, or the name fails `is_audio_name`/`is_video_name` | `"<n>" is not an audio file` / `"<n>" is not a video file` (today's texts) |

The extension check comes after the existence checks, so a typo gives "No file found" rather
than a type complaint. A FIFO is rejected by `metadata` **before** `File::open`, so opening never
blocks. Symlinks to regular files are allowed (metadata follows them), as with the dialog.
Everything later (empty file, not media, no audio track, over 1 GiB, over 15 minutes) is
reported by the existing job with today's "Selected file …" texts.

### 2.4 IPC

- `pick_import_file(kind: ImportKind) -> Result<Option<PickedFile>, String>` with
  `PickedFile { path: String, name: String }` (serde, the TS type mirrors it). It opens the same
  dialog as today (same `DialogKind`, title, filter and start dir = home) and returns the full
  path as UTF-8. A path that is not valid UTF-8 gives `Err("The chosen file's path has
  characters Calliope cannot show; rename the file or its folder")`. It does no other
  validation (`import_path` does it). The existing log line `dialog kind=import-audio|import-video
  result=…` is unchanged.
- `import_path(kind: ImportKind, path: String, events: Channel<ImportEvent>) -> Result<JobSnapshot, String>`
  (`do_import_path(st, import, kind, &path, home, sink)` for tests):
  1. if `import.is_active()` → `BUSY_MESSAGE`;
  2. `local_source::resolve`, and on failure log `calliope: import path kind=<k> result=rejected
     reason=<reason>` and return `Err(message)`;
  3. on success log `calliope: import path kind=<k> result=accepted`, then call
     `import.start_file(&st.current_root(), file_kind, &path, sink)` (unchanged; it also checks
     the tools: no ffmpeg → today's error, no job).
- `import_file` is removed. Both new commands go into `generate_handler!`, `COMMANDS` and
  `capabilities/main.json` (the static test in `tests/frontend.rs` checks that the three match).
  They are `async` with `spawn_blocking`, as the pattern requires.
- Frontend log lines (new): `calliope-ui: import path-picked kind=<k>`, and the existing
  `import error stage=ui message=<m>` for rejected paths (via `showError`).

### 2.5 Security: why a typed path is acceptable here

The 2026-10-05 rule exists so that a compromised webview can't read out or overwrite arbitrary
files. `import_path` is the only command that takes a path. It:
- only reads: metadata, an open for reading, ffprobe and ffmpeg with
  `-protocol_whitelist file,pipe`;
- refuses anything that isn't a regular file with an audio or video extension;
- never returns file contents to the webview. The decoded audio goes only to the owner's own
  edge-AI server, as FLAC.

The worst a compromised webview could do is have a media file's audio uploaded to the
configured server. That is no worse than what Browse already allows once the user picks a
file. Every other command keeps taking tokens. This is recorded in `docs/architecture.md` as a
narrow exception, not a change to the rule.

## 3. Tasks

Each task leaves `npm test` green (it runs in `src/calliope-gui/`). The GUI e2e tests (task 6)
are the only ones that need the new UI and the commands together. Between tasks 2 and 6 the
`#[ignore]`d GUI tests are temporarily out of date: they don't run in `npm test`, and task 6
fixes them.

### Task 1: `local_source.rs`: typed-path normalisation and checks
- **files**: `src/calliope-gui/src/local_source.rs` (new), `src/calliope-gui/src/main.rs`
  (`mod local_source;`), `src/calliope-gui/src/picker.rs` (move the two extension lists and
  `is_audio_name`/`is_video_name` out, replacing them with
  `pub use crate::local_source::{AUDIO_EXTENSIONS, VIDEO_EXTENSIONS, is_audio_name, is_video_name};`)
- **does**: implements §2.3 exactly (the API, the order of steps and checks, the reasons and the
  exact messages). Depends only on std and `url`, with no crate-internal imports, so acceptance
  tests can `#[path]`-include it. Module tests (with `tempfile`) cover:
  - every row of the table, including a FIFO (`mkfifo`, which must return in < 1 s), a dangling
    symlink, a `chmod 000` file (skipped when running as root), a directory named `x.mp3`, and
    `notes.txt` as audio;
  - `song.mp4` as audio and `song.mp3` as video;
  - normalisation: whitespace, single and double quotes, `~`, `~/a b.mp3` with home given and with
    home `None`, `~bob/x.mp3`, `file:///tmp/a%20b.mp3`, `file://otherhost/x.mp3`, a relative
    `a.mp3`, an embedded `\n` or `\0`, and a 4097-byte path;
  - a symlink to a regular file is accepted.
- **done when**: `cargo test -p calliope-gui local_source` passes; `npm test` is green (the
  existing picker tests still pass through the re-export).
- **routine: no** (security-relevant input validation)

### Task 2: IPC: `pick_import_file` and `import_path` replace `import_file`
- **files**: `src/calliope-gui/src/ipc.rs`, `src/calliope-gui/src/gui.rs`,
  `src/calliope-gui/build.rs`, `src/calliope-gui/capabilities/main.json`
- **does**: implements §2.4. `PickedFile` is a `Serialize` struct. `do_pick_import_file(st,
  kind, home)` and `do_import_path(st, import, kind, path, home, sink)` hold the logic; the
  `#[tauri::command]` wrappers only call them. In the `ipc.rs` tests, replace
  `import_file_cancel_and_wrong_extension` and `import_file_without_tools_is_an_error_not_a_job`
  with tests that check:
  - `pick` cancel → `None`, and a pick → the path and name come back verbatim;
  - `import_path` with a wrong extension → today's message and no job;
  - a missing path → `No file found at …` and no job (`import.snapshot().is_none()`);
  - a valid path but no tools → the ffmpeg error and no job;
  - `~/song.mp3` with `home` = the temp base resolves (the no-tools error proves it got past
    `resolve`);
  - busy → `BUSY_MESSAGE`.
- **done when**: `cargo test -p calliope-gui ipc` passes; the static command-list test in
  `tests/frontend.rs` passes; `npm test` is green.
- **routine: no** (IPC surface and ACL)

### Task 3: frontend IPC wrappers
- **files**: `src/calliope-gui/src/ui/lib/ipc.ts`, `src/calliope-gui/src/ui/lib/ipc-import.test.ts`
- **does**: removes `importFile` and adds:
  ```ts
  export interface PickedFile { path: string; name: string }
  export const pickImportFile = (kind: ImportKind): Promise<PickedFile | null> =>
    invoke<PickedFile | null>('pick_import_file', { kind });
  export const importPath = (
    kind: ImportKind, path: string, onEvent: (e: ImportEvent) => void,
  ): Promise<JobSnapshot> =>
    invoke<JobSnapshot>('import_path', { kind, path, events: channel(onEvent) });
  ```
  In `ipc-import.test.ts`, replace every `import_file`/`importFile` use with the two new
  commands, following the existing pattern in that file (the command-name list, the argument
  shape `{ kind }` and `{ kind, path }`, and `null` from a cancelled pick). `import-state.svelte.ts`
  still imports `importFile` until task 4, so in this task also change its `browse()` to call
  `pickImportFile(kind)` and, when it returns a pick, `importPath(kind, pick.path, applyEvent)`
  (this keeps today's behaviour and type-checks; task 4 replaces it).
- **done when**: `npm run check` and `npm run test:ui` pass; `npm test` is green.
- **routine: yes**

### Task 4: the path row in the source page
- **files**: `src/calliope-gui/src/ui/lib/import-state.svelte.ts`,
  `src/calliope-gui/src/ui/components/import/SourcePage.svelte`,
  `src/calliope-gui/src/ui/views/ImportView.test.ts`
- **does**: implements §2.2. In `imp`, add `path: ''`. Split the work into `browse(kind)`
  (pick only: fills `imp.path`, logs `path-picked`, returns `true` on a pick so the component can
  focus Extract) and `extractPath(kind)` (empty check, then `importPath` and `adopt`, the same
  `starting`/`eventSeq` handling as today's `browse`). `chooseSource` clears `path` when the kind
  changes, and `resetImport`/`importAnother` clear it. The Svelte row follows §2.2: give Extract
  `bind:this` to focus it, and run Enter in the box through `extractPath`. In vitest, replace the
  `import_file` cases with cases that check:
  - for both kinds: the box, Browse and Extract are shown, and the URL box is not;
  - Browse calls `pick_import_file` only; a pick fills the box and focuses Extract; a cancelled
    pick leaves the box unchanged; neither starts an import;
  - typing a path and pressing Extract or Enter calls `import_path` with `{ kind, path }` (the
    text as typed);
  - an empty box shows the exact empty message with no IPC call;
  - a rejected path (the mock throws `No file found at /x/y.mp3`) shows that text in the alert,
    keeps the box text and leaves the controls enabled;
  - switching audio → video clears the box;
  - the existing cases keep passing through the new flow ("no audio track" message, 15-minute
    message, straight to ready → edit pane), started by typing or by Browse then Extract.
- **done when**: `npm run check`, `npm run test:ui` and `npm test` pass.
- **routine: no** (state flow with event ordering)

### Task 5: headless acceptance for the two criteria through a typed path
- **files**: `src/calliope-gui/tests/acceptance_existing_track_import.rs` (new)
- **does**: follows the structure of `tests/acceptance_stem_extraction.rs`:
  - `#[path]`-includes `fsutil`, `track_meta`, `repository`, `import_tmp`, `media`, `download`,
    `tools`, `stem_audio`, `import_job` and `local_source`;
  - uses the real `calliope-stems` on `127.0.0.1:0` with the stub separator in `ok` mode, and the
    real ffmpeg/ffprobe on fixtures copied into a temp dir (copy the rig parts you need; don't
    import from another test file).

  For each of `tagged.mp3` (audio) and `with-audio.mp4` (video), it types the path in three
  forms: plain absolute, `"quoted"` with spaces in a copied name (`My Song.mp3`), and
  `file://…%20…`. For each form it runs `local_source::resolve` → `start_file` → ready →
  `start_extraction` with the prefilled edits → saved. It then asserts:
  - a new `tracks/<id>/` exists with `type: "stem"`, `schema_version: 2`, 6 `stems/*.flac`
    listed in `track.json`, and the prefilled title/band;
  - `import-tmp/` is empty;
  - the source file is byte-identical;
  - a sibling "outside" folder is unchanged.

  It also checks:
  - `no-audio.mp4` resolves fine but the job fails with `Selected file no-audio.mp4 has no audio
    track`;
  - a video with two audio streams of different lengths (generated in the test with ffmpeg:
    a 3 s and a 5 s sine) imports its **first** audio stream: at `ready`, the job's
    `import-tmp/file-*/audio.flac` lasts 3 s (FLAC STREAMINFO via `claxon` or ffprobe);
  - rejected paths start no job and create nothing under the root.
- **done when**: `cargo test -p calliope-gui --test acceptance_existing_track_import` passes;
  `npm test` is green.
- **routine: no**

### Task 6: GUI e2e: helpers, typed-path tests, visual review
- **files**: `src/calliope-gui/tests/gui_import_e2e.rs`,
  `src/calliope-gui/tests/acceptance_stem_extraction_gui.rs`
- **does**:
  - updates both `browse()` helpers to the new flow: from the focused radio group press
    `Tab Tab Return` (box, Browse), wait for the Rust line `dialog kind=import-<k> result=picked`
    and `calliope-ui: import path-picked`, then press `Return` (Extract has the focus). Check
    the key sequence on display `:1`; disabled controls are skipped by Tab.
  - adds a `type_path(&self, p)` helper (`Tab`, `ctrl+a`, type, `Return`).
  - adds the tests below.
  - every test keeps the existing guards: loopback-only namespace, stub server, not the `cpal`
    backend, no `csp-violation`, and the outside fingerprint.
  - views every screenshot (`gui-shot` PNGs in `target/gui-shots/`) and checks:
    - the box, Browse and Extract sit on one row with consistent heights;
    - the placeholder is readable;
    - the error text is in the accent colour directly under the row;
    - nothing is clipped at 1280x800.

  New tests:
  - `import_typed_audio_path`: type `<media>/tagged.mp3`, then edit pane → Extract → saved stem
    track (`check_stem_track`, `check_disk`). Screenshots `import-path-audio` (box filled) and
    `import-path-audio-edit`.
  - `import_typed_video_path`: the same with `<media>/with-audio.mp4` (screenshot
    `import-path-video`).
  - `import_typed_path_errors`: for a missing path, a folder (`<media>`) and `<media>/tagged.mp3`
    typed under Local Video File, expect `import error stage=ui message=No file found at …`,
    `… is a folder, not a file` and `"tagged.mp3" is not a video file`; no `import job=` line and
    an unchanged repository. Screenshot `import-path-error`.
  - `import_browse_fills_path`: Browse (scripted pick) fills the box. Screenshot
    `import-path-browsed` shows the full path; then Return imports.
  - The existing tests (`import_local_audio`, `import_local_video`, `import_video_without_audio`,
    `import_sixteen_minute_file_refused`, `import_keep_original`, `import_cancel_while_working`,
    `import_close_window_during_extraction`, the empty-stem tests and the acceptance GUI tests)
    pass with the updated helper.
- **done when**: `DISPLAY=:1 npm run test:gui` passes twice in a row; the screenshots have been
  reviewed and any layout fixes applied (with `npm test` still green).
- **routine: no** (GUI timing and visual review)

### Task 7: user docs
- **files**: `src/calliope-gui/README.md`
- **does**: in the usage section on importing, replace the "Browse starts the import" wording
  with: choose Local Audio File or Local Video File, type or paste the full path (`/…`, `~/…`, a
  quoted path or a `file://` link) or use Browse to fill it in, then press Extract or Enter; a
  video's first audio track is used. Add one line listing the path messages (no file, folder,
  not a regular file, cannot read, wrong type).
- **done when**: `npm test` is green (the README is not tested; a reviewer reads the text).
- **routine: yes**

## 4. Test strategy

- **Unit (Rust)**:
  - `local_source` covers every normalisation rule and check, with real temp files, a FIFO, a
    dangling symlink and permissions;
  - `ipc.rs` tests cover the two new commands with the scripted picker and `Tools::default()`
    (no tools).
- **Unit (frontend)**: `ImportView.test.ts` and `ipc-import.test.ts` with `mockIPC` cover the row,
  focus, the empty check, error display, clearing and the IPC arguments.
- **Headless acceptance**: `tests/acceptance_existing_track_import.rs` runs both criteria end to
  end. The edge-AI server is the real `calliope-stems` binary on 127.0.0.1 with
  `tests/support/stub-separator`; ffmpeg/ffprobe are the local tools on generated fixtures. No
  network, no real model, no yt-dlp.
- **GUI (display `:1`)**: `tests/gui_import_e2e.rs` types paths through `xdotool`, uses the
  scripted picker for Browse (`CALLIOPE_E2E_DIALOG_ANSWERS`, `e2e-hooks`), runs in the
  loopback-only namespace with the stub server, and asserts on log lines and the files on disk.
  Screenshots are reviewed by the agent. No test opens an audio device (`NullBackend`, already
  asserted).
- **Whole suite**: `cd src/calliope-gui && npm test` (headless, all crates); GUI:
  `cd src/calliope-gui && DISPLAY=:1 npm run test:gui`.
- No test reads the user's home: `~` expansion is tested with an injected home, and GUI tests
  only type absolute paths under `target/`.

## 5. Deployment

Single machine: only `calliope-gui` on the laptop changes (`npm run build:app` in
`src/calliope-gui/`). `calliope-stems` on archserver is unchanged, so no redeploy is needed.

## 6. Manual checks (owner, on the laptop)

1. Import → Stem Extraction → Local Audio File. The path box, Browse and Extract are on one row.
2. Browse, choose a real mp3: the box shows its full path and nothing starts yet; press Enter →
   the edit pane opens with the tags filled in; Extract → the stems appear in the Library.
3. Copy a file in your file manager (Ctrl+C) and paste it into the box (Ctrl+V); also try
   `~/Music/<file>` typed by hand. Both start the import.
4. Type a path with a typo: "No file found at …" appears under the box; fix it and continue.
5. Local Video File: paste the path of a real video → the edit pane, then stems for its audio.
   If you have a video with several audio tracks (e.g. a concert with commentary), check that
   the music track is the one used (see open question 1).
6. A silent video → "Selected file <name> has no audio track".
7. Keyboard only: Tab reaches the box, Browse and Extract in that order; Enter in the box
   starts the import.

## 7. Acceptance mapping

| Criterion | Tasks | Tests |
|---|---|---|
| Req 1: audio selected → path box + Browse (system dialog) | 2, 3, 4, 6 | vitest row/Browse cases; GUI `import_browse_fills_path`, `import_local_audio`, screenshots |
| Req 2: video selected → path box + Browse | 2, 3, 4, 6 | vitest (both kinds); GUI `import_local_video`, `import_typed_video_path` |
| Req 3: audio from video, then same as audio | existing (`media::to_flac`), 5 | acceptance video cases incl. two-stream video; existing `video_file_flow_…`, `a_video_without_audio_…` |
| AC1: audio + valid path → stems in the Library | 1, 2, 4, 5, 6 | `acceptance_existing_track_import` (mp3, three path forms); GUI `import_typed_audio_path` |
| AC2: video + valid path → stems of its audio in the Library | 1, 2, 4, 5, 6 | `acceptance_existing_track_import` (mp4, three path forms); GUI `import_typed_video_path` |
| Invalid paths (implied by "valid path") | 1, 2, 4, 6 | `local_source` tests; ipc tests; vitest rejected-path case; GUI `import_typed_path_errors` |

## 8. Open questions

1. **Videos with several audio tracks.** AC2 says "the stems corresponding to the audio
   tracks" (plural). Today's import, kept by this plan, uses the **first** audio track of the
   video and makes one stem track from it. Is that what you want? Alternatives: let you choose
   the audio track (language/commentary) before extraction, or make one stem track per audio
   track. Both are larger and would be a follow-up increment.
