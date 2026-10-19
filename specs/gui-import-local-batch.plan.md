# Plan: Local import: smarter Browse and whole-directory import

Spec: `specs/gui-import-local-batch.md`. Base: `ab99cc6` on `spec/gui-import-local-batch`.

## 1. Summary

Browse (Local Audio File, Local Video File and the new Local Directory) gets a starting folder
chosen in Rust: the typed path's folder, else the folder remembered from the last accepted local
import (a new settings field), else home. A fourth source, **Local Directory**, lists the audio
files directly inside a folder and imports them one after another in a single **batch job**
inside the existing import job slot. Each file goes through the same preparation, extraction and
save code as a single Local Audio File import, but the edit pane is skipped. The page shows
"File N of M: name" with the current file's progress and the files already done. At the end a
summary lists the saved tracks and the failed files.

## 2. Design

### 2.1 What is reused, what is new

| Part | Change |
|---|---|
| `src/local_source.rs` | **new pure functions**: `browse_start` (the dialog's starting folder), `resolve_dir` (typed folder checks), `list_audio_files` (audio files directly inside, sorted), `no_audio_message` |
| `src/settings.rs` | new field `last_import_dir: Option<PathBuf>` + setter; lenient load |
| `src/picker.rs` | `DialogKind::ImportDirectory` (`import-directory`); `ScriptedPicker` records each request's `start_dir`; `impl Picker for Arc<P>` (so tests can keep a handle on the scripted picker) |
| `src/import_job.rs` | **refactor without behaviour change**: `prepare_file` (from `run_file`, everything except `set_ready`) and `extract_cleaned` (the body of the `start_extraction` thread: `run_extract` plus the staging/server-job cleanup); new types `BatchSnapshot`, `BatchItem`, `BatchOutcome`; `Phase::Finished`, `SourceKind::Directory`; `JobSnapshot.batch`; events `BatchFile`, `BatchItem`, `Finished` |
| `src/import_batch.rs` (new) | child module of `import_job` (`#[path = "import_batch.rs"] mod batch;`, so it can use the private `Ctx`, `Job`, `Stop`, …): `ImportState::start_batch` and the batch thread |
| `src/ipc.rs`, `src/gui.rs`, `build.rs`, `capabilities/main.json` | `pick_import_file(kind, typed)` (kind may be `directory`); new command `import_directory(path, events)`; `import_path` and `import_directory` remember the folder |
| frontend | `ipc.ts` types and wrappers; `import-state.svelte.ts` (directory source, batch events, steps); `SourcePage.svelte` (fourth option, directory row, notice); new `BatchProgress.svelte`; the batch summary in `ImportView.svelte`; `ExtractProgress.svelte` gets an `embedded` prop |
| `tests/support/stub-separator` | new mode `seq:<m1>,<m2>,…` (run n uses the n-th mode; the last one repeats) |
| unchanged | server, protocol, `media.rs`, `import_tmp.rs`, repository, the single-file and URL flows |

### 2.2 Spec open questions (decided with the proposed defaults)

1. **The edit pane is skipped in a directory import.** Each file's tags give the metadata
   (`media::edits_from_tags(probe, name)`, which already falls back to the file name without its
   extension as the title, then "Untitled"). The tracks are edited later in the Library.
2. **There is one shared "last location"** for audio files, video files and directories:
   `settings.last_import_dir`.
3. **The dialog only opens the folder.** It never preselects the typed file.

### 2.3 The dialog's starting folder (requirements 1-2)

The pure function is in `local_source.rs` and only reads metadata:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartFrom { Typed, Last, Home, None }
impl StartFrom { pub fn as_str(self) -> &'static str }   // "typed" | "last" | "home" | "none"

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseStart { pub dir: Option<PathBuf>, pub from: StartFrom, pub forget_last: bool }

pub fn browse_start(typed: &str, last: Option<&Path>, home: Option<&Path>) -> BrowseStart
```

The rules, in this order:
1. **Typed.** `normalise(typed, home)`, the same function Extract uses, so `~/`, quotes and
   `file://` work the same way and relative paths are refused. If it succeeds and
   `fs::metadata` succeeds (symlinks are followed): a directory gives itself, anything else gives
   its `parent()` (when that is not empty). Any error falls through **silently**.
2. **Last.** If `last` is `Some(p)` and `fs::metadata(p)` is a directory, the result is `p`.
   Otherwise `forget_last = true` and the rules go on. The last location is only checked when
   rule 1 didn't apply.
3. **Home.** `home`, or `StartFrom::None` with `dir: None` (the dialog then uses its own
   default).

`ipc::do_pick_import_file(st, store, kind, typed, home)` calls `browse_start` with
`store.get().last_import_dir`. If `forget_last` is true it calls `store.set_last_import_dir(None)`
and logs `calliope: import last-location forgotten` (a save error is only logged). It logs
`calliope: dialog start kind=<import-audio|import-video|import-directory> from=<typed|last|home|none> dir=<path|none>`,
then opens:
- audio / video: `pick_file(FileReq { start_dir, .. })`, as today;
- directory: `pick_folder(FolderReq { kind: ImportDirectory, title: "Choose a folder of songs", start_dir })`.

The return value is the same `PickedFile { path, name }` (for a folder, `name` is its last
component). The same non-UTF-8 error is used, with "file" replaced by "folder" for directories.

**Remembering (requirement 2).** "Accepted" means the job started (`start_file` /
`start_batch` returned `Ok`). Rejected paths, busy refusals and missing tools don't count. Then
`remember_location(store, folder)` saves `last_import_dir` and logs
`calliope: import last-location dir=<path>`. A save error is logged and is never an import error.
- File import (typed or browsed): `folder = abs.parent()`.
- Directory import: `folder = dir` itself (owner answer to open question 1 in section 8: the
  next Browse opens `/music/band/album1` after importing `/music/band/album1`).

The path is stored as normalised, not canonicalised, like the paths sent to ffmpeg.
`settings.json` example:
`{"theme":"dark","repository_root":null,"edge_ai_url":"http://archserver:8765","keep_original":false,"last_import_dir":"/music/rock"}`.
On load, a value that is not an absolute path string falls back to `None` with a warning, like
`repository_root`. A folder that is missing on load is only forgotten when Browse consults it.

### 2.4 Directory checks and listing (requirements 4-5)

```rust
pub fn resolve_dir(input: &str, home: Option<&Path>) -> Result<PathBuf, PathProblem>
pub fn list_audio_files(dir: &Path) -> Result<Vec<PathBuf>, PathProblem>
pub fn no_audio_message(dir: &Path) -> String   // "Directory <dir> has no audio files"
```

- `resolve_dir`: `normalise` first. An `empty` result becomes
  `"Enter the path of a folder or choose one with Browse"` (reason `empty`). Then, in this
  order: missing → `missing`, `"No folder found at <p>"`; other metadata errors → `unreadable`
  (the same texts as for files); not a directory → `not-folder`, `"<p> is not a folder"`.
  Symlinks to folders are accepted.
- `list_audio_files`: `read_dir` (an error gives `unreadable`, `"Cannot read <p>"` plus
  ` (permission denied)` when that is the cause). Only direct entries are used, never
  subdirectories. An entry is kept when its name doesn't start with `.` (hidden files, e.g.
  `._song.mp3`; see open question 2), `is_audio_name(name)` holds (case-insensitive, the
  `AUDIO_EXTENSIONS` of Local Audio File; video files are skipped) and `fs::metadata(path)` (symlinks
  followed) is a regular file. Entries that can't be read are skipped. **Order**: by
  `name.to_lowercase()`, then by the raw name as a tie-break, so `a.flac` comes before `B.mp3`
  before `c.ogg`. Non-UTF-8 names are compared lossily and imported with their real path.
- There is no limit on the number of files.

`ipc::do_import_directory` does: busy → `BUSY_MESSAGE`; `resolve_dir` (rejected:
`calliope: import path kind=directory result=rejected reason=<r>`, `Err(message)`);
`list_audio_files`; an empty list gives `reason=no-audio` and `Err(no_audio_message(dir))`;
then `start_batch`. On `Ok` it logs `... result=accepted files=<n>` and remembers the location.
The messages appear under the path box through the existing `imp.error` (`#import-error`).

### 2.5 The batch job (requirements 4, 6-8, 10)

**One job in the existing slot.** The batch is one `Job` with `SourceKind::Directory` (label =
the folder's name). It is "active" until it ends, so another import, a root change and a second
batch are refused with the existing messages. The batch shares one `Cancel`. The thread runs the
files in order and reuses the single-file code for each:

```text
start_batch(root, dir, files, edge_ai_url, keep_original, sink)
  pre-checks (nothing starts on error): files not empty; edge-AI URL set (NOT_CONFIGURED_MESSAGE);
  ffmpeg + ffprobe usable; not busy; check_root; ImportTmp::open; clean_stale; clean_stale_staging
  snapshot: phase Preparing, batch {dir, files, current: Some(0), items: []}; launch(dir = PathBuf::new())
thread, for i in 0..n:
  if cancelled -> stop
  reset the per-file snapshot fields (downloaded, total, sent, progress, stems_*, dropped, metadata,
    duration_s), batch.current = Some(i)
  log "import batch job=<j> file=<i+1>/<n> name=<name> result=started"; emit BatchFile{index:i,total:n,name}
  phase(Preparing)
  dir_i = tmp.new_file_dir("audio-file", name, now)            (error -> item Failed{prepare})
  job.dir = dir_i
  prepare_file(ctx, dir_i, path, name) -> (audio, edits, duration)
  snapshot.metadata = edits; phase Uploading + emit Uploading{0,0}
  extract_cleaned(ctx, dir_i, audio, edits, url, keep)          (removes dir_i on success)
  Ok  -> item Saved{track, dropped}
  Failed(e) -> remove dir_i (if it exists), item Failed{stage, message}
  Cancelled -> remove dir_i, log "... result=cancelled", stop
  push the item to the snapshot, log it, emit BatchItem{item}
end: not cancelled -> phase Finished, current None, over; log "import batch job=<j> finished saved=<a> failed=<b>";
       emit Finished{batch}
     cancelled     -> phase Cancelled, current None, over; log "import batch job=<j> cancelled saved=<a> skipped=<k>";
       emit Cancelled{back_to: Source}
```

- **Per-file failures don't end the batch (requirement 7).** `prepare_file` / `extract_cleaned`
  errors become `BatchOutcome::Failed`. The batch never emits the single-import `Failed`,
  `Ready` or `Saved` events, so the frontend can't confuse them with a single import.
- **Cancel (requirement 8).** It stops the current file, using the paths the single import already
  has: ffmpeg is killed, the upload is aborted, the server job is deleted, and staging is abandoned
  by `extract_cleaned`. The rest are skipped, and tracks already committed stay. Cancel is allowed
  in every phase. Pressed while a file is `saving`, the commit finishes (that file is listed as
  saved) and the batch stops before the next file. `shutdown` (app exit) works unchanged, because
  it cancels the shared `Cancel`.
- **Temp space.** There is one `import-tmp/file-<uuid>/` per file, created when the file starts
  and removed when it ends, so at most one exists at a time. Nothing is left after a batch,
  whatever its outcome.
- **Logs (requirement 10).** The per-file phase lines `calliope: import job=<j> phase=…` and the
  stem lines are the same as for single imports. The batch lines are listed in 2.8.

**Types** (Rust, serialised for IPC):

```rust
pub struct BatchSnapshot {
    pub dir: String,            // the folder as resolved (display form)
    pub files: Vec<String>,     // file names in import order
    pub current: Option<usize>, // 0-based index of the file being imported; None once ended
    pub items: Vec<BatchItem>,  // finished files, in order
}
pub struct BatchItem { pub index: usize, pub name: String, #[serde(flatten)] pub outcome: BatchOutcome }
#[serde(tag = "result", rename_all = "lowercase")]
pub enum BatchOutcome {
    Saved { track: TrackRecord, dropped: Vec<DroppedStem> },
    Failed { stage: Stage, message: String },
}
// JobSnapshot gains:  #[serde(skip_serializing_if = "Option::is_none")] pub batch: Option<BatchSnapshot>
// Phase gains Finished ("finished"); SourceKind gains Directory ("directory")
// ImportEvent gains:
//   #[serde(rename = "batch-file")] BatchFile { index: usize, total: usize, name: String }
//   #[serde(rename = "batch-item")] BatchItem { item: BatchItem }
//   Finished { batch: BatchSnapshot }                       // "finished"
```

The JSON of single jobs doesn't change, because `batch` is omitted when `None`. Event examples:
`{"phase":"batch-file","index":0,"total":2,"name":"a.flac"}`,
`{"phase":"batch-item","item":{"index":0,"name":"a.flac","result":"saved","track":{…},"dropped":[]}}`,
`{"phase":"batch-item","item":{"index":1,"name":"b.mp3","result":"failed","stage":"prepare","message":"…"}}`.

### 2.6 IPC

| Command | Change |
|---|---|
| `pick_import_file(kind: "audio"\|"video"\|"directory", typed: string \| null)` | new Rust enum `PickKind` (lowercase); `typed` is `Option<String>` (null/absent = empty). The return value is unchanged |
| `import_path(kind, path, events)` | unchanged signature; it remembers the location once accepted |
| `import_directory(path, events) -> JobSnapshot` (**new**) | 2.4. It is added to `generate_handler!` (`src/gui.rs`), `COMMANDS` (`build.rs`) and `capabilities/main.json` (`allow-import-directory`) |

**Security note** (extends the 2026-10-10 typed-path decision): `import_directory` is the second
command that takes a path. It only reads: it checks that the path is a folder, lists the folder's
direct entries and passes the audio files to the same read-only preparation as `import_path`.
`pick_import_file`'s `typed` text is only used to choose the dialog's starting folder
(`metadata` only). No path is ever written, created or deleted.

TS (`src/ui/lib/ipc.ts`): `PickKind = ImportKind | 'directory'`;
`pickImportFile(kind: PickKind, typed: string)`; `importDirectory(path, onEvent)`; `JobPhase` gains
`'finished'`; `JobSource.kind` gains `'directory'`; `JobSnapshot.batch?: BatchSnapshot`;
`BatchSnapshot`, `BatchItem` (the intersection of `{index, name}` with the two `result` variants)
and the three events; `Settings.last_import_dir?: string | null` (optional in TS; unused by the frontend).

### 2.7 UI

**Source page.** A fourth radio, `Local Directory` (value `directory`), comes **last**, so the
keyboard order and the existing GUI tests' `Down` counts stay the same. Its row has the same
layout as the file row: input `id="import-path"`, `aria-label="directory path"`, placeholder
`/path/to/folder`; Browse (outline; `browse('directory')`; on a pick it fills the box and focuses
Extract); Extract (Enter in the box does the same; `extractDirectory()`). An empty box shows
`Enter the path of a folder or choose one with Browse`. Every Browse (audio, video, directory)
sends the current box text as `typed`. Under the controls, besides the existing `#import-error`,
a new `#import-notice` (`role="status"`, muted text) shows the cancel note (below). It is cleared
by any source change, Browse, Extract or Import another.

**Batch progress (`BatchProgress.svelte`, step `extract` when `job.batch` is set).** Heading
"Importing a directory". `<p role="status" data-testid="batch-current">File 3 of 8: song.mp3</p>`
(1-based). Below it is the current file's progress: phase `preparing` shows "Preparing audio..."
with an indeterminate bar; every other phase uses `<ExtractProgress embedded />`, which hides its
heading and its own Cancel when `embedded`. Then come the Cancel button (visible in every phase; it
is the **only focusable control** on the page, so `cancel()` in the GUI tests (heading click, Tab,
Return) still works) and "Done" (`aria-label="Files done"`), a list with one line per finished
file: `a.flac: saved as <title>` (plus ` (dropped empty stems: piano, other)`) or
`b.mp3: failed: <message>` (accent colour). The list scrolls (`max-h-48 overflow-y-auto`), so a
long batch doesn't push Cancel off screen at 1024x640.

**Summary (step `done` with `job.batch`, phase `finished`).** It is a section
`aria-label="Import finished"`:
- `<p role="status">Imported <s> of <n> files from <dir>.</p>`;
- "Saved tracks" list: `<title>`, plus `: dropped empty stems: a, b` when some were dropped;
- "Failed files" list (only when there are any): `<name>: <message>` in the accent colour;
- buttons **Show in Library** and **Import another track** (the same labels as after a single
  import).

Show in Library selects the **first saved track** (the tree is expanded as today). With no saved
track it only opens the Library.

**Cancel.** The page goes back to the idle source page (Local Directory, path kept), with the
notice `Directory import cancelled: <s> of <n> files saved.` and the Library reloaded.

**State (`import-state.svelte.ts`).**
- `step()`: when `job.batch` is set, `finished` → `done`, `cancelled` → `source`, and every
  other phase → `extract`.
- `footerText()`: during a batch it is `File <i> of <n>: <phase text>`; `finished` →
  "Import finished".
- `applyEvent`:
  - `batch-file`: creates `job.batch` if a stub job has none (`files` of length `total`),
    then sets `files[index] = name` and `current = index`; logs `batch file=<i+1>/<n> name=<name>`.
  - `batch-item`: pushes the item; logs `batch item=<i+1> result=saved id=<id> dropped=<a,b|none>`
    or `batch item=<i+1> result=failed stage=<s> message=<m>`; `void load()` after a saved item.
  - `finished`: `job.batch = e.batch`, `phase = 'finished'`; logs
    `batch finished saved=<a> failed=<b>`; `void load()`.
  - `cancelled` with a batch: sets the notice, `job = null`; logs `batch cancelled saved=<a> of=<n>`;
    `void load()`.
- `adopt()` copies `snap.batch` into a stub job that has none.
- `attach()` handles `finished` (calls `load()`) and running batches (step `extract`).

### 2.8 Log lines (new)

Rust (`calliope: `):
- `dialog start kind=<k> from=<typed|last|home|none> dir=<p|none>`
- `import last-location dir=<p>`, `import last-location forgotten`
- `import path kind=directory result=accepted files=<n>`
- `import path kind=directory result=rejected reason=<empty|invalid|relative|missing|unreadable|not-folder|no-audio>`
- `import batch job=<j> files=<n>`
- `import batch job=<j> file=<i>/<n> name=<name> result=started|saved id=<id> dropped=<a,b|none>|failed stage=<s> message=<m>|cancelled`
- `import batch job=<j> finished saved=<a> failed=<b>`, `import batch job=<j> cancelled saved=<a> skipped=<k>`

`dialog kind=import-directory result=picked|cancelled` comes from the existing `log_dialog`.

Frontend (`calliope-ui: import `):
- `source=directory`, `path-picked kind=directory`
- `batch file=<i>/<n> name=<name>`
- `batch item=<i> result=saved id=<id> dropped=<…>`, `batch item=<i> result=failed stage=<s> message=<m>`
- `batch finished saved=<a> failed=<b>`, `batch cancelled saved=<a> of=<n>`

`phase=<phase> job=<job>` is also logged for the new events (`phase=batch-file` …), as for
every other event.

### 2.9 Test stand-in: `seq:` stub mode

`tests/support/stub-separator`: if the mode starts with `seq:`, the script counts its runs in
`<work dir>/stub-runs` (the work dir is `$PWD/../..`, where `stub-mode` lives; one line, a
number, created at 0). Run n (1-based) uses the n-th comma-separated mode, and the last mode
repeats. `seq:ok,fail,ok` therefore fails the second file of a batch only, and `seq:ok,hang`
hangs on the second file. The server runs one job at a time, so the counter needs no locking.

## 3. Tasks

Rust tasks end with `cargo clippy -p calliope-gui --all-targets -- -D warnings` clean, and frontend
tasks with `npm run check` clean (both in `src/calliope-gui/`). Implementers run only the tests
named in their task. The orchestrator runs `npm test` and `npm run test:gui`. **Extend existing
tests instead of adding near-duplicates**: reuse the `Rig`s of `src/import_job_tests.rs`,
`tests/acceptance_existing_track_import.rs` and `tests/gui_import_e2e.rs`. **Keep waits short**:
`ImportState` polls every 10-50 ms in Rust tests; the 5 s fixtures are the shortest audio; GUI tests
wait on log lines, never fixed sleeps beyond `settle()`. Never commit in a routine task. Never
open a real dialog, never use the real `calliope-stems` on `0.0.0.0:8765` or any other host.

### Task 1: Stub separator `seq:` mode
- **files:** `src/calliope-gui/tests/support/stub-separator`, `src/calliope-gui/tests/support/README.md`
- **does:**
  - In the header comment, after the `boundary` line, add:
    `#   seq:M1,M2,...  run n (counted in <work dir>/stub-runs) uses the n-th mode; the last one repeats`
  - Directly after the line `mode=${mode:-ok}`, insert exactly:
    ```bash
    if [[ $mode == seq:* ]]; then
      runs_file="$PWD/../../stub-runs"
      runs=0
      [[ -f $runs_file ]] && IFS= read -r runs <"$runs_file" || true
      runs=$((runs + 1))
      echo "$runs" >"$runs_file"
      IFS=, read -r -a seq_modes <<<"${mode#seq:}"
      idx=$((runs - 1))
      ((idx >= ${#seq_modes[@]})) && idx=$((${#seq_modes[@]} - 1))
      mode=${seq_modes[$idx]}
    fi
    ```
  - `tests/support/README.md`: in the stub-separator description, add one sentence:
    "`seq:ok,fail,ok` picks a mode per run (counted in `<work dir>/stub-runs`), so one file of a
    batch can fail or hang."
- **done when:** `bash -n src/calliope-gui/tests/support/stub-separator` succeeds, and this check
  (from the repository root) prints `3 0 0`, because run 1 fails, run 2 is ok and the last mode
  repeats:
  ```bash
  S=$PWD/src/calliope-gui/tests/support/stub-separator F=$PWD/src/calliope-gui/tests/fixtures/import/untagged.flac
  T=$(mktemp -d) && mkdir -p $T/w/jobs/j && echo 'seq:fail,ok' >$T/w/stub-mode && cd $T/w/jobs/j &&
    r=""; for n in 1 2 3; do "$S" "$F" $T/o$n m >/dev/null 2>&1; r="$r $?"; done; echo $r; cd - >/dev/null
  ```
  `cargo test -p calliope-stems --test conformance` still passes.
- **routine:** yes

### Task 2: Settings field `last_import_dir`
- **files:** `src/calliope-gui/src/settings.rs`
- **does:**
  - `Settings`: after `keep_original`, add
    ```rust
    /// Folder of the last accepted local import (file or directory), where Browse starts; `None` = none yet.
    pub last_import_dir: Option<PathBuf>,
    ```
  - `load`: after the `keep_original` match, add (the same pattern as `repository_root`):
    ```rust
    match obj.get("last_import_dir") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::String(d)) if Path::new(d).is_absolute() => {
            s.last_import_dir = Some(PathBuf::from(d));
        }
        Some(v) => eprintln!(
            "calliope: invalid last_import_dir {v} in {} (not an absolute path string); using none",
            path.display()
        ),
    }
    ```
  - `SettingsStore`: after `set_keep_original`, add `pub fn set_last_import_dir(&self, dir: Option<PathBuf>) -> std::io::Result<Settings>`
    with the same body as `set_keep_original`, but setting `next.last_import_dir = dir;`.
  - Tests: in `json_shape`, the expected string ends with `"keep_original":false,"last_import_dir":null}`.
    New test `last_import_dir_roundtrip_and_lenient`: save with `Some("/music/rock")`, then `load`
    equals it. A file with `"last_import_dir":"rel/x"` or `5` loads as `None` and keeps
    `"theme":"light"`. `SettingsStore::set_last_import_dir(Some(..))` persists across
    `SettingsStore::open` of the same path, and `set_last_import_dir(None)` clears it.
- **done when:** `cargo test -p calliope-gui --lib settings::` passes.
- **routine:** yes

### Task 3: Picker: directory kind, recorded start folders, `Arc` picker
- **files:** `src/calliope-gui/src/picker.rs`
- **does:**
  - `DialogKind`: add the variant `ImportDirectory`, with `as_str` → `"import-directory"`, and add
    it to the `parse` list.
  - After the `Picker` trait, add:
    ```rust
    impl<P: Picker + ?Sized> Picker for std::sync::Arc<P> {
        fn pick_file(&self, req: &FileReq) -> Option<PathBuf> { (**self).pick_file(req) }
        fn pick_folder(&self, req: &FolderReq) -> Option<PathBuf> { (**self).pick_folder(req) }
        fn save_file(&self, req: &SaveReq) -> Option<PathBuf> { (**self).save_file(req) }
    }
    ```
  - `ScriptedPicker`: add the field `starts: Mutex<Vec<(DialogKind, Option<PathBuf>)>>`
    (initialised empty in `new`) and
    `pub fn starts(&self) -> Vec<(DialogKind, Option<PathBuf>)> { self.starts.lock().unwrap_or_else(|e| e.into_inner()).clone() }`.
    In its `Picker` impl, each of the three methods first pushes `(req.kind, req.start_dir.clone())`
    into `starts`, then answers as today.
  - Test `scripted_records_start_dirs`: two `pick_file` calls (start `Some("/m")` and `None`) and
    one `pick_folder` call of kind `ImportDirectory` (answers `import-audio CANCEL`,
    `import-audio CANCEL`, `import-directory /m/album`). `starts()` returns the three pairs in
    order, and the folder call returns `Some("/m/album")`. Through
    `let p = Arc::new(ScriptedPicker::new(f)); let b: Box<dyn Picker> = Box::new(p.clone());`,
    a call on `b` shows up in `p.starts()`.
- **done when:** `cargo test -p calliope-gui --lib picker::` passes.
- **routine:** yes

### Task 4: `local_source`: starting folder, directory checks, audio listing
- **files:** `src/calliope-gui/src/local_source.rs`
- **does:** `StartFrom`, `BrowseStart`, `browse_start`, `resolve_dir`, `list_audio_files` and
  `no_audio_message`, exactly as in 2.3 and 2.4 (pure: std only, metadata/read_dir only).
- **tests** (in the module, temp dirs, injected home, never the real `~`):
  - `browse_start`:
    - an existing file → its folder, `Typed`;
    - an existing folder → itself;
    - `~/x.mp3` with home = a temp dir holding `x.mp3` → home, `Typed`;
    - `'<quoted path>'` and `file://` forms work;
    - a missing path, a relative path, an empty or blank box → `Last` when `last` exists;
    - `last` missing → `forget_last` true and `Home`;
    - no home → `None`/`dir: None`;
    - a dangling symlink → falls through;
    - when the typed path wins, `forget_last` stays false even for a missing `last`.
  - `resolve_dir`: each reason and exact message (`empty` with the folder text, `relative`,
    `missing` "No folder found at …", `not-folder` on a file, `unreadable` on a `chmod 000`
    folder (skipped as root)); a symlink to a folder is accepted; `~` expands.
  - `list_audio_files`, on a folder with `b.mp3`, `a.flac`, `C.OGG`, `cover.jpg`, `.hidden.mp3`,
    `clip.mp4`, `sub/c.mp3`, a folder named `x.mp3`, a FIFO `pipe.wav`, a symlink `link.wav` →
    `b.mp3` and a dangling `dead.mp3`: the result is `[a.flac, b.mp3, C.OGG, link.wav]`. An
    empty folder gives `[]`. An unreadable folder (`chmod 000`, skipped as root) gives
    `unreadable`.
  - `no_audio_message(Path::new("/m/x"))` == `"Directory /m/x has no audio files"`.
- **done when:** `cargo test -p calliope-gui --lib local_source::` passes.
- **routine:** no

### Task 5: Import job: helpers and batch types (no behaviour change)
- **files:** `src/calliope-gui/src/import_job.rs`, `src/calliope-gui/src/import_job_tests.rs`
- **does:**
  - `fn prepare_file(ctx, dir, path, name) -> R<(PathBuf, TrackEdits, Option<f64>)>`, which is
    `run_file` without `set_ready`. `run_file` becomes `prepare_file` + `set_ready`.
  - `fn extract_cleaned(ctx, dir, audio, edits, url, keep) -> R<(TrackRecord, Vec<DroppedStem>)>`
    is the `start_extraction` thread's body after the first `Uploading` emit: `run_extract` plus
    abandoning staging and deleting the server job. The thread becomes
    `emit(Uploading{0,0}); let r = extract_cleaned(..); finish_extract(&ctx, r);`.
  - Types from 2.5: `BatchSnapshot`, `BatchItem`, `BatchOutcome` (derive `Debug, Clone, PartialEq, Serialize`),
    `Phase::Finished`, `SourceKind::Directory` (`#[serde(rename = "directory")]`), `JobSnapshot.batch`
    (`skip_serializing_if = "Option::is_none"`, `None` in `new_snapshot`), and the `ImportEvent`
    variants `BatchFile`, `BatchItem`, `Finished` with their serde names. Nothing emits them yet;
    put `#[allow(dead_code)]` only where clippy needs it until task 6.
  - Extend `event_json_shapes` with the exact JSON of the three new events and with a single-job
    snapshot that has **no** `batch` key.
- **done when:** `cargo test -p calliope-gui --lib import_job::` passes (all existing tests
  unchanged), and clippy is clean.
- **routine:** no

### Task 6: The batch runner
- **files:** `src/calliope-gui/src/import_batch.rs` (new), `src/calliope-gui/src/import_job.rs`
  (declares `#[path = "import_batch.rs"] mod batch;`), `src/calliope-gui/src/import_job_tests.rs`
- **does:** `ImportState::start_batch(&self, root, dir: &Path, files: Vec<PathBuf>, edge_ai_url: Option<&str>, keep_original: bool, sink: Sink) -> Result<JobSnapshot, String>`
  and the thread from 2.5 (pre-checks, per-file loop, items, end states, logs from 2.8).
  `Job.dir` follows the current file's folder, and `discard` after the end is a no-op apart from
  forgetting the job.
- **tests** (in `import_job_tests.rs`, extending `Rig` with `fn batch(&self, names: &[&str], server: &TestServer) -> Result<JobSnapshot, String>`
  that copies fixtures into `sources/batch/` under the given names, and polling at 50 ms):
  - `batch_imports_each_file_in_order`: `a.flac` (untagged.flac) and `b.mp3` (tagged.mp3) with
    the `ok` server. The events, deduplicated by phase, run batch-file(0) → preparing → uploading
    → … → saving → batch-item(saved) → batch-file(1) → … → finished. There are 2 tracks, with
    titles `a` and `Glass Harbour`. The final snapshot is `Finished` with `current: None` and 2
    items. `import-tmp/` has no job folders, and the sources are byte-identical.
  - `batch_failures_go_on`: `1.flac`, `2.mp3` (not-audio.mp3), `3.ogg` → the outcomes are
    saved / failed (stage `prepare`) / saved. With the server in `seq:ok,fail,ok` on three valid
    files → the second fails with stage `server`, and no `Failed`/`Ready`/`Saved` single events
    are emitted.
  - `batch_cancel_on_the_second_file`: `seq:ok,hang`. Wait for the snapshot `current == Some(1)`
    and phase `Working`, then `cancel`. The outcome is `Cancelled{back_to: Source}`, phase
    `Cancelled` and over; 1 track and 1 item; the server log shows the DELETE; there are no
    staging or import-tmp folders.
  - `batch_refusals_start_nothing`: an empty list, no URL (`NOT_CONFIGURED_MESSAGE`), and busy
    (a second `start_batch` / `start_file` while running) → `Err`, with no new job.
  - `shutdown_during_a_batch`: `seq:hang`; `shutdown(3 s)` returns true, and the server job is
    deleted.
- **done when:** `cargo test -p calliope-gui --lib import_job::` passes (the new tests take
  well under 30 s in total), and clippy is clean.
- **routine:** no

### Task 7: IPC: start folder, remembering, `import_directory`
- **files:** `src/calliope-gui/src/ipc.rs`, `src/calliope-gui/src/gui.rs`, `src/calliope-gui/build.rs`,
  `src/calliope-gui/capabilities/main.json`
- **does:**
  - `PickKind { Audio, Video, Directory }`.
  - `do_pick_import_file(st, store, kind: PickKind, typed: &str, home)` as in 2.3.
  - `remember_location(store, folder)`.
  - `do_import_path(st, import, store, kind, path, home, sink)`: remembers the folder after `Ok`.
  - `do_import_directory(st, import, store, path, home, sink)` as in 2.4. It passes
    `store.get().edge_ai_url` and `keep_original` to `start_batch` and remembers the location
    after `Ok`.
  - Tauri commands: `pick_import_file(app, kind: PickKind, typed: Option<String>)`, and
    `import_directory(app, path: String, events)` (async, `blocking`).
  - Registration in the three places (`ipc::import_directory` after `ipc::import_path`,
    `"import_directory"` after `"import_path"`, `"allow-import-directory"` after
    `"allow-import-path"`).
  - Update the existing ipc tests to the new signatures and add:
    - `pick_start_dir_rules`: the `env()` picker is wrapped in `Arc` (change `Env` to keep an
      `Arc<ScriptedPicker>` and box a clone into `RepoState`). It covers typed file → folder,
      typed dir, empty → last, last deleted → home and the setting cleared, and a directory kind
      using `pick_folder` with kind `ImportDirectory`; the start dirs are checked with `starts()`.
    - `accepted_imports_remember_the_folder`: a rejected path doesn't change the setting; with the
      fake ffmpeg/ffprobe of `import_path_while_a_job_runs_is_busy`, an accepted path sets its
      parent; `do_import_directory` on `<base>/album` sets `<base>/album`.
    - `import_directory_rejections`: missing, a file, an empty folder → the exact messages, and no
      job.
- **done when:** `cargo test -p calliope-gui --lib ipc::` and `cargo test -p calliope-gui --test frontend`
  (command lists in sync) pass, and clippy is clean.
- **routine:** no

### Task 8: TS IPC types and wrappers
- **files:** `src/calliope-gui/src/ui/lib/ipc.ts`, `src/calliope-gui/src/ui/lib/ipc-import.test.ts`
- **does:**
  - `Settings`: add `last_import_dir?: string | null;` after `keep_original` (optional, so the existing test fixtures that
    build `Settings` objects still type-check; the frontend doesn't use it).
  - After `export type ImportKind = 'audio' | 'video';` add `export type PickKind = ImportKind | 'directory';`.
  - `JobPhase`: add `| 'finished'`. `JobSource.kind`: add `| 'directory'`.
  - Add the types:
    ```ts
    export type BatchItem = { index: number; name: string } & (
      | { result: 'saved'; track: TrackRecord; dropped: DroppedStem[] }
      | { result: 'failed'; stage: JobStage; message: string });
    export interface BatchSnapshot { dir: string; files: string[]; current: number | null; items: BatchItem[] }
    ```
  - `JobSnapshot`: add `batch?: BatchSnapshot | null;`.
  - `ImportEvent`: add
    `| { phase: 'batch-file'; index: number; total: number; name: string } | { phase: 'batch-item'; item: BatchItem } | { phase: 'finished'; batch: BatchSnapshot }`.
  - `pickImportFile = (kind: PickKind, typed: string): Promise<PickedFile | null> => invoke<PickedFile | null>('pick_import_file', { kind, typed });`
  - After `importPath`, add
    `export const importDirectory = (path: string, onEvent: (e: ImportEvent) => void): Promise<JobSnapshot> => invoke<JobSnapshot>('import_directory', { path, events: channel(onEvent) });`
  - `ipc-import.test.ts`: the `pickImportFile` calls pass a second argument (`'audio', '/m/x.mp3'`),
    and the expected payload becomes `{ kind: 'audio', typed: '/m/x.mp3' }`. Add
    `'import_directory'` to the list of command names, and add one call
    `importDirectory('/m/album', () => {})` whose payload has `path: '/m/album'`.
  - Fix the compile errors in other files caused only by the new `pickImportFile` argument, by
    passing `imp.path` in `import-state.svelte.ts` (`await pickImportFile(kind, imp.path)`).
- **done when:** `npm run check` is clean and `npx vitest run src/ui/lib/ipc-import.test.ts` passes
  (in `src/calliope-gui/`).
- **routine:** yes

### Task 9: Import state: directory source and batch events
- **files:** `src/calliope-gui/src/ui/lib/import-state.svelte.ts`, `src/calliope-gui/src/ui/lib/import-state.test.ts` (new)
- **does:**
  - `SourceChoice` gains `'directory'`. `browse(kind: PickKind)` sends `imp.path`.
  - `extractDirectory()`: the empty-box message, then `importDirectory` + `adopt`.
  - `imp.notice`.
  - `step`, `footerText`, `jobRunning`, `applyEvent`, `adopt`, `attach`, `showInLibrary` and
    `importAnother` / `resetImport` as in 2.7, with the log lines from 2.8.
- **tests** (`import-state.test.ts`, mockIPC like `ImportView.test.ts`, driving `applyEvent`):
  - a batch-file event on a stub job creates the batch; the steps go
    `extract` → `done` on finished;
  - the item log lines and texts;
  - `footerText` "File 2 of 3: Working...";
  - cancelled → step `source`, job null, notice
    `Directory import cancelled: 1 of 3 files saved.`;
  - `showInLibrary` picks the first saved item, or only switches the view when none was saved;
  - `adopt` keeps the event's batch;
  - `browse` sends `typed`;
  - an empty directory box → the folder message, and no IPC call.
- **done when:** `npx vitest run src/ui/lib/import-state.test.ts` passes, and `npm run check` is clean.
- **routine:** no

- **Orchestrator note (after tasks 5-6):** after Cancel the snapshot has phase `Cancelled`, `over`,
  `batch.current = None` and keeps `batch.files` and `batch.items`; the `Cancelled{back_to: Source}`
  event carries no counts, so build "<s> of <n> files saved" from the items. A Cancel that arrives
  while the last file is saving lets it save and the batch ends `Finished`. `start_batch` also
  refuses an empty list ("There are no files to import") and non-absolute paths.

### Task 10: Source page, batch progress and summary
- **files:** `src/calliope-gui/src/ui/components/import/SourcePage.svelte`,
  `src/calliope-gui/src/ui/components/import/BatchProgress.svelte` (new),
  `src/calliope-gui/src/ui/components/import/ExtractProgress.svelte`, `src/calliope-gui/src/ui/views/ImportView.svelte`,
  `src/calliope-gui/src/ui/views/ImportView.test.ts`, `src/calliope-gui/src/ui/acceptance_qa_typed_path.test.ts`,
  `src/calliope-gui/src/ui/acceptance_import.test.ts`
- **does:** the UI of 2.7 (CSP rules: no static `style=`, Tailwind classes only).
- **tests:**
  - In `ImportView.test.ts`, the radio test lists four options. Then:
    - Local Directory Browse sends `{kind: 'directory', typed}` and fills the box;
    - Extract sends `import_directory`;
    - a rejected directory shows the message under the box;
    - the batch events render "File 2 of 3: b.mp3", the done list, and a Cancel that calls
      `cancel_import`;
    - the summary shows the saved titles with their dropped stems, the failed names with their
      messages, and both buttons;
    - a cancelled event shows the notice on the source page.
  - In `acceptance_qa_typed_path.test.ts:82` and the other exact `pick_import_file` payload
    assertions, add `typed` (the box text at the time of Browse; `''` when empty).
- **done when:** `npm run test:ui` passes, and `npm run check` and `npm run build` are clean.
- **routine:** no

- **Orchestrator note (after tasks 8-10), UI names for tasks 11-12:** radio "Local Directory" (last); directory
  row: input `#import-path` `aria-label="directory path"`, Browse, Extract (Enter also starts); empty box error
  "Enter the path of a folder or choose one with Browse" in `#import-error`. Cancel notice `#import-notice`
  (`role=status`). Progress section `aria-label="Directory import progress"`, heading "Importing a directory",
  `data-testid="batch-current"` "File i of n: name", embedded progress, Cancel, `<ul aria-label="Files done">`. Summary
  `aria-label="Import finished"`, "Imported s of n files from <dir>.", lists "Saved tracks"/"Failed files", Show in
  Library, Import another track. Footer "File i of n: <phase>" / "Import finished". Frontend log lines: `import
  batch file=i/n name=…`, `import batch item=i result=saved|failed …`, `import batch finished saved=a failed=b`,
  `import batch cancelled saved=a of=n`. The Back button stays rendered (disabled) on the batch page.

### Task 11: Headless acceptance tests
- **files:** `src/calliope-gui/tests/acceptance_existing_track_import.rs`
- **does:** extends the file (the local-import acceptance suite, with its real stub server
  `RealServer::start(mode)` and `Rig`). The new tests use `calliope_gui::ipc::{RepoState, do_pick_import_file, do_import_path, do_import_directory}`
  with `RepoState::new(root, Box::new(picker.clone()))` (an `Arc<ScriptedPicker>`), a temp
  `SettingsStore` and an `ImportState` polling at 50 ms. They are named after the criteria:
  - `ac_browse_opens_the_typed_files_folder`, `ac_browse_opens_the_typed_directory`;
  - `ac_browse_opens_the_last_location_also_after_restart`: accept a file in `<tmp>/music/rock/`,
    drop and reopen the `SettingsStore` from the same file, then Browse with an empty box and
    with a missing path → start `<tmp>/music/rock`;
  - `ac_browse_opens_home_when_nothing_was_imported`;
  - `ac_directory_browse_uses_the_folder_dialog` (kind `ImportDirectory`, `PickedFile.path` = the chosen folder);
  - `ac_directory_import_order_and_ignored_entries`: `b.mp3`, `a.flac`, `cover.jpg`, `sub/c.mp3`;
    the batch-file events give `a.flac` then `b.mp3`; 2 stem tracks; `c.mp3` and `cover.jpg`
    are never probed (sources byte-identical, `import-tmp` clean);
  - `ac_directory_without_audio_files`: the exact message and no job;
  - `ac_second_of_three_fails`: server `seq:ok,fail,ok`; tracks 1 and 3 are saved and the
    finished batch lists the second file with its server message;
  - `ac_cancel_on_file_two_of_three`: `seq:ok,hang`; first track kept, nothing else, phase
    `Cancelled` (idle), the server job deleted;
  - `ac_progress_names_the_current_file`: the snapshot during file 2 has `current == Some(1)`,
    `files[1]` = the name, and a per-file phase (`working`).
- **done when:** `cargo test -p calliope-gui --test acceptance_existing_track_import` passes.
- **routine:** no

### Task 12: GUI end-to-end and visual review
- **files:** `src/calliope-gui/tests/common/mod.rs`, `src/calliope-gui/tests/gui_import_e2e.rs`
- **does:**
  - `import_dirs`: also creates `media/album/` (`a.flac` = untagged.flac, `b.mp3` = tagged.mp3,
    `cover.jpg` = the bytes `not an image`, `sub/c.mp3` = tagged.mp3), `media/three/` (`1.flac`
    untagged, `2.ogg` tagged.ogg, `3.mp3` tagged.mp3) and `media/no-songs/` (`cover.jpg`,
    `notes.txt`). They are copied before `Rig::new` snapshots `media`, so `check_disk` proves
    they are untouched.
  - `Rig::open_source(3)` = Local Directory: `Down Down Down space`, waiting for `source=directory`.
  - `Rig::new_with_settings(test, stub, answers, extra: serde_json::Value)` writes
    `settings.json` before the start (`start_stems_app` keeps the extra fields).
  - **No position updates are needed in the existing GUI tests.** They drive the page with the
    keyboard (a heading click at (700, 55), then Tab and arrow keys), and the new radio is the last
    one, so the `Down` counts for URL/audio/video are unchanged. Re-run the whole
    `gui_import_e2e` and `acceptance_stem_extraction_gui` binaries to confirm.
  - New tests:
    - `import_browse_start_folders`: seeded `last_import_dir = {media}`, Local Audio File. Browse
      with an empty box gives `dialog start kind=import-audio from=last dir={media}`; typing
      `{media}/tagged.mp3` gives `from=typed dir={media}`; `{media}/album` gives
      `from=typed dir={media}/album`; `{media}/nope.mp3` gives `from=last` (each answer
      `CANCEL`). Then Local Directory: Browse answers `import-directory {media}/album`
      (`path-picked kind=directory`; screenshot `import-source-directory`). Typing
      `{media}/no-songs` + Enter shows `Directory {media}/no-songs has no audio files`, and
      `{media}/tagged.mp3` shows `{media}/tagged.mp3 is not a folder`; no `import job=` line.
    - `import_directory_batch`: no seeded setting. Local Directory, then Browse with an empty box
      gives `from=home dir=<HOME>`, answering `{media}/album`; Return. The test waits for
      `batch file=1/2 name=a.flac`, takes the screenshot `import-batch-progress` during
      `phase=working`, then waits for `batch item=1 result=saved`, `batch file=2/2 name=b.mp3` and
      `batch finished saved=2 failed=0` (screenshot `import-batch-summary`). It checks 2 new
      tracks (titles `a`, `Glass Harbour`), `check_disk`, and `settings.json` `last_import_dir`
      = `{media}/album` (owner answer: the directory itself). Then Import another, Local Audio File,
      Browse with an empty box → `from=last dir={media}/album`.
    - `import_directory_one_fails`: stub `seq:ok,fail,ok` on `{media}/three` (typed path);
      `batch item=2 result=failed stage=server`; `finished saved=2 failed=1`; screenshot
      `import-batch-failed-summary`; 2 new tracks.
    - `import_directory_cancel`: stub `seq:ok,hang` on `{media}/three`. After
      `batch item=1 result=saved` and `batch file=2/3` plus `phase=working`, `r.cancel()` →
      `batch cancelled saved=1 of=3`, the source page again (screenshot
      `import-batch-cancelled`), 1 new track, the server DELETE logged, `check_disk`,
      `assert_no_children`.
  - **Visual review** (look at the PNGs, both themes at 1280x800 and once at 1024x640 for the
    progress and summary pages): the four radios are aligned; "File N of M: name" is above the
    per-file progress; the done list scrolls and Cancel stays visible; the summary lists are
    readable; the failed texts use the accent colour. Fix the layout until the screenshots look
    right.
- **done when:** `npm run test:gui -- import_` passes for `gui_import_e2e` (and
  `acceptance_stem_extraction_gui` is unchanged and green), and the screenshots are reviewed.
- **routine:** no

### Task 13: README and UI guide
- **files:** `src/calliope-gui/README.md`, `docs/ui.md`
- **does:**
  - README "Import and stem extraction", step 1: after the sentence ending "then press Extract
    or Enter.", add:
    "Browse opens in the folder of the path in the box (a file's folder, or the folder itself),
    else in the folder of your last local import, else in your home folder. **Local Directory**
    imports every audio file directly inside a folder (not its subfolders, not videos), one after
    another in file-name order, each with its own tags (the file name is the title when there is
    none) and without the edit step. The page shows `File N of M: name`, the current file's
    progress and the files done. A file that fails is listed and the next one starts. Cancel
    stops the current file and skips the rest; tracks already saved stay. At the end a summary
    lists the saved tracks and the failed files. A folder without audio files gives
    `Directory <dir> has no audio files`."
  - "Settings files": the settings.json line lists
    `(theme, repository folder, edge-AI address, keep-original, last import folder)`.
  - `docs/ui.md`, after the `(gui-stem-extraction, task 22)` bullet, add a bullet:
    "- (gui-import-local-batch) Import source page: four radios (URL, Local Audio File, Local
    Video File, Local Directory; the new one last so the keyboard order stays). A directory
    import shows `File N of M: name`, the current file's progress, a scrolling list of the files
    done and one Cancel (the only focusable control); the summary lists the saved tracks (with
    dropped stems) and the failed files (accent colour), then Show in Library (the first saved
    track) and Import another track. A cancelled batch returns to the source page with a status
    line."
- **done when:** `npm test` passes (README/doc changes break nothing). `git diff --stat` shows
  only these two files, and both end with a newline.
- **routine:** yes

## 4. Test strategy

- **Unit (Rust):**
  - `local_source` (start-folder rules, directory checks, listing/order/filters on temp dirs with
    FIFOs, symlinks and `chmod 000`, with an injected home);
  - `settings` (the field, lenient load, persistence);
  - `picker` (the directory kind, recorded starts);
  - `import_job_tests` (the batch thread against the **real `calliope-stems` on 127.0.0.1:0
    with the stub separator**, including `seq:` modes, polling at 50 ms);
  - `ipc` tests (start folders through `do_pick_import_file` with the recording
    `ScriptedPicker`, remembering, rejections).
- **Headless acceptance:** `tests/acceptance_existing_track_import.rs` maps each acceptance
  criterion to an `ac_*` test (section 7). App restart = reopening the `SettingsStore` from the
  same file.
- **Frontend:** vitest + mockIPC (`import-state.test.ts`, `ImportView.test.ts`, `ipc-import.test.ts`,
  the typed-path acceptance file).
- **GUI e2e:** `tests/gui_import_e2e.rs` on private X displays. It uses the scripted picker
  (`CALLIOPE_E2E_DIALOG_ANSWERS`, kind `import-directory` added), so a **real dialog never
  opens**. The starting folder is asserted from the `calliope: dialog start` log line. The real
  `calliope-stems` with the stub separator runs inside `unshare -rn` on 127.0.0.1:8765 of the
  private network namespace (never the host's server or archserver). Screenshots go to
  `target/gui-shots/` for visual review.
- **No test** opens the real file/folder dialog or an audio device, reaches the LAN, runs the
  model, or uses symphonia.
- **Commands** (in `src/calliope-gui/`): `npm test` (the whole headless suite, all crates);
  `npm run test:gui` (GUI tests, parallel on private displays); `DISPLAY=:1 npm run test:gui:serial`
  as the alternative.

## 5. Deployment

Only the laptop app changes (`calliope-gui`, built with `npm run build:app`). `calliope-stems`
on archserver is unchanged; the stub separator is test-only. `settings.json` gains one optional
field, and older builds ignore it (unknown fields are ignored).

## 6. Manual checks (owner, on the laptop with the real server)

1. **Real file dialog, typed file.** Import > Stem Extraction > Local Audio File, type the full
   path of a song in `~/Music/<band>/<album>/`, press Browse: the system dialog opens in that
   album folder. Cancel.
2. **Typed folder, invalid text.** Type `~/Music/<band>` and press Browse: the dialog opens in
   that folder. Type `~/nothing-here` and press Browse: no error, and the dialog opens in your
   home (or the last folder, once you have imported something).
3. **Last location across restarts.** Import one song from `~/Music/<band>/<album>/`, then quit
   Calliope. Start it again, choose Local Video File with an empty box and press Browse: the
   dialog opens in the album folder.
4. **Folder dialog.** Choose Local Directory and press Browse: a *folder* chooser opens, in the
   last location. Pick an album folder: its path fills the box.
5. **Real batch.** With the real `calliope-stems` on archserver set in Settings, Extract an album
   folder with 3-4 songs (and a cover image or a subfolder, if it has one). Check that the page
   shows "File 1 of N: …" with Working… progress and the files done grow; that each song ends
   up as its own stem track in the Library with its tags; and that the cover and the subfolder
   songs are not imported. The summary lists the tracks and dropped stems, and Show in Library
   opens the first one.
6. **A failing file.** Put a text file renamed to `broken.mp3` into a copy of the folder and run
   it again: the summary lists `broken.mp3` with its reason, and the other songs are saved.
7. **Cancel.** Start a batch of 3+ songs and press Cancel while the second one is "Working…".
   The page returns to Select source with "Directory import cancelled: 1 of N files saved."; the
   first track is in the Library, and archserver's journal (`journalctl --user -u calliope-stems`)
   shows the job deleted.
8. **Empty folder.** Extract a folder with only images: "Directory … has no audio files" appears
   under the box, and nothing starts.

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests |
|---|---|---|
| Path box holds an existing audio file → the dialog opens in its folder | 4, 7, 9, 10 | `local_source` browse_start tests; `ac_browse_opens_the_typed_files_folder`; GUI `import_browse_start_folders` |
| Path box holds an existing directory → the dialog opens there | 4, 7 | browse_start tests; `ac_browse_opens_the_typed_directory`; GUI `import_browse_start_folders` |
| Empty or missing path, earlier import from /music/rock → /music/rock, also after a restart | 2, 4, 7 | settings tests; `pick_start_dir_rules`; `accepted_imports_remember_the_folder`; `ac_browse_opens_the_last_location_also_after_restart`; GUI `import_browse_start_folders` (seeded = loaded at start-up) and `import_directory_batch` (remembered after the batch) |
| Nothing imported, empty box → home | 4, 7 | browse_start tests; `ac_browse_opens_home_when_nothing_was_imported`; GUI `import_directory_batch` (`from=home`) |
| Local Directory + Browse → the folder dialog; the choice fills the box | 3, 7, 8, 9, 10 | `scripted_records_start_dirs`; `ac_directory_browse_uses_the_folder_dialog`; ImportView test; GUI `import_browse_start_folders` |
| b.mp3, a.flac, cover.jpg, sub/c.mp3 → a.flac then b.mp3, each its own stem track; the others ignored | 4, 5, 6, 7 | `list_audio_files` test; `batch_imports_each_file_in_order`; `ac_directory_import_order_and_ignored_entries`; GUI `import_directory_batch` |
| No audio files → the message, no import | 4, 7, 10 | `import_directory_rejections`; `ac_directory_without_audio_files`; GUI `import_browse_start_folders` (`no-songs`) |
| 3 files, the second fails → the first and third saved; the summary lists the second with its error | 1, 6, 9, 10 | `batch_failures_go_on`; `ac_second_of_three_fails`; ImportView summary test; GUI `import_directory_one_fails` |
| Cancel on file 2 of 3 → the first kept, the second and third not imported, idle page | 1, 6, 9, 10 | `batch_cancel_on_the_second_file`; `ac_cancel_on_file_two_of_three`; import-state cancel test; GUI `import_directory_cancel` |
| While running: "File N of M: name" plus the current file's progress | 6, 9, 10 | `ac_progress_names_the_current_file`; import-state/ImportView tests; GUI `import_directory_batch` (screenshot during `working`) |
| (req. 10) every started/finished/failed/cancelled file is logged | 6 | the log lines of 2.8, asserted in the GUI tests (`calliope: import batch …` lines in the app log) |

## 8. Open questions (answered by the owner 2026-10-19: 1 the directory itself, 2 and 3 as recommended)

1. **Which folder is remembered after a directory import?** The spec says "the folder of the last
   … directory". **Owner answer (2026-10-19): the directory itself** (not the recommended
   parent).
2. **Hidden files in a directory** (names starting with `.`, e.g. macOS `._song.mp3` resource
   files). *Recommended:* skip them like the subdirectories, since they are almost never songs
   and would fail as unreadable.
3. **After Cancel.** The spec asks for "an idle state". *Recommended:* return to the source page
   (Local Directory, path kept) with the status line `Directory import cancelled: <s> of <n>
   files saved.`, rather than the summary page.
