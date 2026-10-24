# Plan: Export info: remember a track's last export to each destination

Spec: `specs/gui-export-info.md`. Base: `caa6db3` on branch `spec/gui-export-info`. Predecessor:
`specs/gui-export.md` / `specs/gui-export.plan.md` (as built; the count-in lead is 5 beats since the
2026-10-24 fix).

## 1. Summary

After a successful export, the export job writes one entry for that kind into the track's
`track.json`, under a new optional `exports` object: `rc5` {memory, at}, `mp3` / `ogg` {path, at}. On
an RC-5 export it also removes the `rc5` entry from any other track that names the same memory. Only
`exports` changes; every other field, including `modified`, stays as it was. The Library's track pane
shows one read-only row per entry. The Export view shows a "Last: ..." line next to each button,
fills the Memory box from the recorded memory, and opens the MP3/OGG folder dialog in the folder of
the recorded file. To keep an edit in progress in the Library working, the `track.json` revision no
longer covers `exports`, and the Library's save keeps the field as it is on disk. A metadata write
that fails doesn't fail the export: the pane reports "The export info wasn't saved: ...".

## 2. Design

### 2.1 What is reused, what is new

| Area | Reused | New / changed |
|------|--------|---------------|
| Metadata | `track_meta::parse` (lenient, v1 migrated in memory), `TrackMeta.extra` (unknown fields kept on save), `fsutil::write_atomic`, `now_rfc3339` | `exports` lives in `extra`, never a typed `TrackMeta` field (2.2); `track_meta::revision` (2.3) |
| Model | – | new pure module `src/export_record.rs`: types, lenient read, JSON-level set/clear (2.2) |
| Repository | `Repository::{track_dir, load, load_record}`, the repository mutex (`RepoState::repo_lock()` / `RepoLock`), `BatchFailure` | `TrackRecord.exports`, `record_export`, `clear_rc5_holders` (2.4) |
| Job | `ExportState::start`'s `on_done` hook (already runs after the rename, before `done`) | the hook takes a `Finished` and returns a `Recorded`; `done` event and snapshot carry the result (2.5) |
| IPC | `do_export_info`, `do_prepare_export`, `do_start_export`, `remember_memory` | `ExportInfo.last`, the memory rule, the MP3/OGG start folder, `record_finished` (2.6). No new command. |
| Rules | `export_rules::{suggest_memory, start_dir}` | `memory_for`, `file_start_dir` (2.6) |
| Frontend | `ex` export state, `loadInfo`, `lib.library`, `TrackPane`'s read-only rows, `export-format.ts` | merging the re-read records, the Memory box refill rule, "Last:" lines, info-error line, Library rows (2.7) |

### 2.2 The record in `track.json` (requirements 1, 3; constraint "older builds")

```json
"exports": {
  "rc5": { "memory": 7, "at": "2026-10-24T18:30:00Z" },
  "mp3": { "path": "/home/me/Music/Band - Title.mp3", "at": "2026-10-24T18:32:05Z" },
  "ogg": { "path": "/home/me/Music/Band - Title.ogg", "at": "2026-10-24T18:33:40Z" }
}
```

- The field is optional and additive, so there is **no schema bump** (`schema_version` stays 2). The
  same reasoning as `position` / `album_position` (decision 2026-10-20): every schema-2 build keeps
  unknown top-level fields in `TrackMeta.extra` and writes them back on save. That is how older
  builds both load and keep a track that has export info. A key that is absent means no kind has an
  entry. A kind without an entry has no key. `exports` is removed once its last entry is cleared,
  so a cleared track looks like one never exported.
- `at` is the export's end time, in canonical RFC 3339 UTC with seconds (`now_rfc3339()`). The
  frontend shows it in local time. `path` is the absolute path of the file written (the target the
  job renamed into place). The RC-5 entry has no base folder (spec 1.1).
- **Not a typed `TrackMeta` field.** `exports` stays in `TrackMeta.extra`. So every existing write
  path (Library save, group patches, the Editor's backing save) keeps it verbatim, exactly as an
  older build does, and nothing on the edit side can change it (requirement 4, "it can't be
  edited"). `SaveTrackRequest` has no such field, and serde ignores unknown request keys.
- **Lenient read** (`export_record::read(Option<&Value>) -> TrackExports`). A wrong value never makes
  the track a problem:
  - `rc5` needs `memory` as an integer 1..=99;
  - `mp3` / `ogg` need `path` as a non-empty absolute path string;
  - `at` is any string (empty when missing or not a string);
  - anything else gives no entry for that kind.
- **Strict, surgical write.**
  - `export_record::set(obj: &mut Map<String, Value>, entry: &ExportEntry, at: &str)` writes only
    `exports.<kind>`. Other keys inside `exports` are kept (a future kind). An `exports` that is not
    an object is replaced.
  - `export_record::clear_rc5(obj) -> bool` removes `exports.rc5`, and `exports` too when it ends
    up empty.
  - The repository applies both to the **raw JSON value of the file**, not to a parsed `TrackMeta`.
    A `TrackMeta` round trip is not lossless: a hand-edited invalid `position` is read as unset and
    then dropped, missing `imported` / `composers` would be written as `""` / `[]`, and a v1 file
    would become schema 2. The spec says nothing but the export info changes.
  - The cost is cosmetic: serde_json's `Value` map is sorted (no `preserve_order`), so the rewritten
    file has its keys in alphabetical order until the next Library save restores the struct order.
- `modified` is **not** changed (spec: "every other field still stay unchanged").

```rust
// src/export_record.rs (pure, std + serde_json)
pub const EXPORTS_KEY: &str = "exports";          // also used by track_meta::revision
#[derive(Debug, Clone, PartialEq, Eq, Serialize)] pub struct Rc5Export { pub memory: u8, pub at: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize)] pub struct FileExport { pub path: String, pub at: String }
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TrackExports { pub rc5: Option<Rc5Export>, pub mp3: Option<FileExport>, pub ogg: Option<FileExport> }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportEntry { Rc5 { memory: u8 }, Mp3 { path: String }, Ogg { path: String } }
pub fn read(v: Option<&Value>) -> TrackExports;
pub fn rc5_memory(obj: &Map<String, Value>) -> Option<u8>;   // = read(obj.get(EXPORTS_KEY)).rc5.memory
pub fn set(obj: &mut Map<String, Value>, entry: &ExportEntry, at: &str);
pub fn clear_rc5(obj: &mut Map<String, Value>) -> bool;
```

`TrackExports` serializes every kind, with `null` for a missing one (`{"rc5": null, "mp3": {...}, "ogg": null}`).

### 2.3 Revision without `exports` (requirement 8)

Today the revision is FNV-1a over the bytes of `track.json`. A Library edit keeps the revision from
when the track was loaded, and its save is refused (`conflict:`) when the file changed. If the
export rewrote the file under this rule, an edit open on the exported track, or on the track whose
RC-5 entry was cleared, would get a "changed on disk" conflict that has nothing to do with the
user's edit.

**Choice: the revision ignores `exports`.**
- `track_meta::revision(bytes) -> String`: parse the bytes as a JSON value. If it is an object,
  remove the top-level `exports`, serialize it compactly (`serde_json::to_vec`: sorted keys,
  `float_roundtrip`) and return the `fnv1a64_hex` of that. Otherwise return `fnv1a64_hex(bytes)`.
- `Repository::load` uses it in place of `fsutil::fnv1a64_hex(&bytes)`. Every revision check
  (save, delete, batch patches, batch delete) goes through `load`, so they all change together.
- Consequences:
  - A change to `exports` alone never conflicts.
  - Any other change still does, including unknown fields.
  - A whitespace-only or key-order-only hand edit no longer counts as a change. That is harmless,
    because the content is identical, and it is also what lets the alphabetical rewrite of 2.2 go
    unnoticed.
  - The frontend treats revisions as opaque, so nothing changes there.
- `save_track` re-reads the file under the lock and applies the edits to what is on disk, so a stale
  draft (loaded before the export) saves with the new `exports` in place. The cleared track behaves
  the same way: it stays cleared.
- Alternatives:
  - Let the frontend patch the draft's revision after an export. Rejected: it would also hide a real
    external change made in between, and it needs the frontend to know which tracks changed.
  - A separate `exports.json`. Rejected: the owner wants the info in `track.json`.

### 2.4 Repository operations (requirements 1-3, 8, 9)

`src/repository.rs`:

- `TrackRecord` gains `pub exports: TrackExports`, filled in `record()` from
  `export_record::read(m.extra.get(EXPORTS_KEY))`. There are three struct literals to extend with
  `exports: Default::default()`: `repository.rs` `record()`, the `export_rules.rs` test helper
  `rec()`, and `tests/acceptance_tracks_repository.rs` (the newer-schema fake).
- `pub fn record_export(&self, id: &str, entry: &ExportEntry, at: &str) -> Result<TrackRecord, String>`:
  1. `track_dir(id)` (a valid id and a real folder);
  2. read the bytes;
  3. `track_meta::parse(&bytes, id)` must succeed: problem tracks and tracks from a newer build are
     never rewritten (`Err("track \"<id>\": <reason>")`);
  4. parse the bytes as a `Value` object and apply `export_record::set`;
  5. `write_atomic(track.json, to_string_pretty + "\n")`; an error becomes
     `io_err("cannot write track.json", e)`;
  6. log `calliope: export recorded id=<id> target=rc5 memory=<n> at=<t>`, or
     `... target=<mp3|ogg> path=<p> at=<t>`;
  7. return `load_record(id)`.
- `pub fn clear_rc5_holders(&self, memory: u8, except: &str) -> ClearOutcome { cleared: Vec<TrackRecord>, failed: Vec<BatchFailure> }`:
  - **Which folders:** the folders of `tracks_dir()` in name order, skipping non-folders, dot/tmp
    names, invalid ids, symlinks (through `track_dir`) and `except`.
  - **Quick check:** read `track.json` and parse it as a `Value`. A folder is a candidate only when
    `rc5_memory == Some(memory)`.
  - **Problem tracks:** a candidate is cleared only if `track_meta::parse` accepts it. Otherwise
    log `calliope: export clear skipped id=<id> reason=<m>` and leave it alone.
  - **Clear:** `clear_rc5`, `write_atomic`, log `calliope: export cleared id=<id> memory=<n>`, then
    push `load_record`.
  - **Failure:** a write failure goes into `failed` (with the title) and is logged as
    `calliope: export clear failed id=<id> memory=<n> message=<m>`. The others go on.
  - **Several holders:** every one is cleared, which repairs a repository after hand edits or an
    earlier failure.
  - **Cost:** one small file read per track, and a full parse only for candidates. That is about a
    millisecond per hundred tracks, once per RC-5 export. No full `scan` is needed: it would also
    stat every listed file.
- Both run under the repository mutex (2.5). Neither touches `modified`, another field or another
  file.

### 2.5 When the info is written: the export job (requirements 1, 2, 8)

`src/export_job.rs`:

```rust
pub struct Finished { pub id: String, pub kind: ExportKind, pub memory: Option<u8>, pub path: PathBuf }
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Recorded { pub root: String, pub records: Vec<TrackRecord>, pub info_error: Option<String> }
pub type OnDone = Box<dyn FnOnce(&Finished) -> Recorded + Send>;
// ExportState::start(.., on_done: OnDone)
// ExportEvent::Done { path, clipped, root: String, records: Vec<TrackRecord>, info_error: Option<String> }
// ExportSnapshot gains `info_error: Option<String>` (None until done).
// planned(token) returns `Option<PlannedRef { id, track_dir, repo_root, kind, memory }>` (was a tuple).
```

- `Work::run` calls `on_done(&Finished{..})` only on success, after the rename and directory sync
  and before `done` is published, which is where it already runs today. A cancelled, refused or
  failed export never reaches it (requirement 2). The `Recorded` goes into the `done` event and the
  snapshot (`info_error`), so a re-attached pane still sees the message.
- `do_start_export` (`src/export_ipc.rs`) builds the hook:
  1. remember the RC-5 memory in the settings (as today);
  2. call `record_finished(&*repo.repo_lock(), &plan.repo_root, finished, &now_rfc3339())`.

  It uses the plan's repository root, so a repository switched during the export still records into
  the track's own repository (the same rule as the tempo map; gui-export fix 4).
- `pub fn record_finished(lock: &dyn RepoLock, root: &Path, f: &Finished, at: &str) -> Recorded` runs
  inside `lock.run(..)`:
  1. **The entry:** `ExportEntry::Rc5 { memory }`, or `Mp3/Ogg { path }`. A path that is not valid
     UTF-8 is not recorded: `info_error` is "The export info wasn't saved: the file's path is not
     valid UTF-8".
  2. **Record it:** `Repository::new(root).record_export(..)`. On `Err(m)`, `info_error` =
     `"The export info wasn't saved: <m>"` and the log gets
     `calliope: export record failed id=<id> message=<m>`.
  3. **RC-5 only:** `clear_rc5_holders(memory, id)` runs **whether or not step 2 worked**, because
     the memory now holds this track's audio either way. Each failure adds
     `The RC-5 info of "<title>" (memory <n>) wasn't cleared: <m>` to `info_error`, joined with a
     space.
  4. **The result:** `records` = the exported track's new record (when step 2 worked) plus the
     cleared tracks' records. `root` = `root.to_string_lossy()`.
- **Partial failure stays visible.** The file is in place and the export shows as done, and the
  missing or stale info is named in the pane and the log. There is no rollback: the file is what
  matters, and the info is a convenience.
- **Locking:**
  - The hook runs on the export thread, outside `ExportState`'s own mutex, and holds the repository
    mutex only for the two small writes and the scan.
  - A Library save waits for it or runs before it. Each one re-reads the file, so neither loses the
    other's change.
  - Another Calliope instance is covered only by the revision check (accepted limit, 2026-10-05).
    An interleaved write there could still drop the info.

### 2.6 IPC and rules (requirements 5, 6, 7)

- `ExportInfo` (`export_info`):
  - gains `last: TrackExports` (the track's record as read now);
  - `memory` = `export_rules::memory_for(rec.exports.rc5.map(|e| e.memory), settings.rc5_memory)`:
    the recorded memory when there is one, else `suggest_memory(last used)` as today.
- `do_prepare_export`, MP3/OGG only:
  - the dialog starts at `export_rules::file_start_dir(recorded, remembered, home)`, where
    `recorded` = the recorded path of **that** kind (MP3 uses the `mp3` entry, OGG the `ogg` one);
  - `file_start_dir` returns `("recorded", parent)` when the recorded file's parent folder exists,
    else `start_dir(remembered, home)`, which gives "last", then "home", then "none" as today;
  - the existing log line gets `from=recorded`;
  - the RC-5 dialog is unchanged (remembered `rc5_dir`).
- The picked folder is still remembered in `export_dir` as before. The file name is still
  `<Band> - <Title>.<ext>`, so the same folder gives the same file and the existing `exists` →
  confirmation flow (requirements 5, 6).
- **No new command.** The Tauri command list, `build.rs` `COMMANDS` and `capabilities/main.json`
  stay as they are.

### 2.7 Frontend (requirements 4, 5, 7, 8)

- **Types (`src/ui/lib/ipc.ts`):**
  - `Rc5Export {memory, at}`, `FileExport {path, at}`,
    `TrackExports {rc5: Rc5Export|null; mp3: FileExport|null; ogg: FileExport|null}`;
  - `TrackRecord.exports?: TrackExports`: Rust always sends it, and it is optional only so that test
    literals stay valid, the pattern `position` already uses;
  - `ExportInfo.last: TrackExports`, `ExportSnapshot.info_error: string | null`;
  - `done` event: `{phase: 'done'; path; clipped; root: string; records: TrackRecord[]; info_error: string | null}`.
- **Format helpers (`src/ui/lib/export-format.ts`, pure):**
  - `formatStamp(iso)` → `YYYY-MM-DD HH:MM` and `formatDay(iso)` → `YYYY-MM-DD`, both in **local
    time** (Date getters), and `''` for an empty or unparseable value;
  - `exportRows(exports?)` → `[{id: 'track-export-rc5', label: 'RC-5', value: 'memory 7 (2026-10-24 21:30)'}, {id: 'track-export-mp3', label: 'MP3', value: '<path> (2026-10-24 21:32)'}, {.. 'OGG' ..}]`,
    one row per kind present, in that order, with no parentheses when `at` is empty, and `[]` when
    there is nothing (requirement 4);
  - `lastText(kind, last)` → `Last: memory 7, 2026-10-24` / `Last: <path>, 2026-10-24` / `''`
    (requirement 7).
- **Library state (`library-state.svelte.ts`):**
  - new `mergeTracks(root, records)`: when `lib.library?.root === root`, replace the records with
    the same id. Nothing else changes: no rescan, and the draft is left alone. This is how the
    Library shows new info even when an edit is in progress: the Library and Export views skip the
    rescan in edit mode.
  - The draft is separate from the record, the revision didn't change (2.3), and `saveEdit` sends
    no `exports`. So an open edit is not disturbed (requirement 8).
- **Export state (`export-state.svelte.ts`):**
  - **On `done`:** set `s.info_error`, call `mergeTracks(e.root, e.records)`, then
    `loadInfo(s.id, true)` (as today).
  - **On a failure:** if `info_error`, log `export info-error message=<m>`.
  - **Memory box rule (requirement 5):** `loadInfo` refills the box when it is called with
    `refill`, when the box is empty, **or when the id differs from the one the box was last filled
    for** (a new `filledFor`, cleared by `resetExport`). So selecting a track shows its recorded
    memory, or "last used + 1" without one. Typing still works, and the typed text survives a
    reload of the same track's info.
  - **Refill race:** a refill requested by a call that loses the `infoSeq` race is kept in a
    `refillPending` flag and applied by the response that wins.
  - **Logging:** after each successful load, log
    `export last id=<id> memory=<box> rc5=<n|none> mp3=<path|none> ogg=<path|none>`. The GUI tests
    read this line.
- **Export pane (`ExportPane.svelte`):**
  - The info-loading effect reads a `$derived` `targetId` string. A merged record object with the
    same id then doesn't reload the info.
  - Each button row gets a muted `lastText(...)` paragraph: `data-testid="export-last-rc5|mp3|ogg"`,
    `min-w-0 break-all text-sm text-muted-foreground`, in the same flex-wrap row, after the Memory
    box for the RC-5.
  - Under the result line, when `ex.snap?.phase === 'done' && ex.snap.info_error`, an alert line
    `data-testid="export-info-error"` in the accent-warning colour (`text-amber-700 dark:text-primary`)
    shows the message as Rust wrote it.
  - The job area keeps its reserved height.
- **Library track pane (`TrackPane.svelte`):**
  - `exportRows(selected?.exports)` adds read-only `Input` rows after "Modified", the same markup
    as the other read-only rows, never editable. They come from the record, not the draft, so edit
    mode doesn't change them.
  - An effect logs `track exports id=<id> rows=<rc5,mp3,ogg|none>` whenever the selected id or its
    row list changes. The GUI tests read this line.

### 2.8 Error and edge cases

| Case | Behaviour |
|------|-----------|
| Export cancelled, refused, failed, overwrite declined | nothing recorded; `track.json` byte-identical |
| Exported track's `track.json` unwritable / track deleted meanwhile / became a problem | file exported, done; `info_error` "The export info wasn't saved: ..."; other holders still cleared |
| Another holder unwritable | exported track recorded; `info_error` names that track; the log says `export clear failed` |
| Another holder is a problem track | skipped, logged, never rewritten |
| Same track re-exported to the same memory | its own entry is updated (`at`); the scan skips it (`except`) |
| Recorded MP3 folder deleted | dialog starts in `export_dir`, else home |
| Repository switched during the export | recorded in the plan's repository; the frontend merges only when the root matches |
| Hand-edited garbage in `exports` | ignored on read (no rows); replaced on the next record of that kind; kept verbatim by Library saves |
| v1 `track.json` | recorded in place, stays v1 (2.2); the record shows the entry |

## 3. Tasks

All tasks are `routine: no`: each one either adds logic to a new or long file, or changes shared
behaviour. (The project notes say the local model breaks new files with logic and edits inside long
files.) The coordinator runs the full `npm test` once per task. Implementers run the tests named
under each task.

### Task 1: `export_record` module and the `exports`-free revision
- files: `src/export_record.rs` (new), `src/track_meta.rs`, `src/lib.rs`
- does:
  - add the types and functions of 2.2 (`read`, `rc5_memory`, `set`, `clear_rc5`, `EXPORTS_KEY`)
    and `track_meta::revision(bytes)` of 2.3;
  - declare `pub mod export_record`;
  - unit tests in `export_record.rs`:
    - lenient read: valid entries; memory 0 / 100 / "7"; a relative path; a missing `at`; a
      non-object `exports`; unknown kinds kept by `set`;
    - `set` replaces only its own kind;
    - `clear_rc5` removes `exports` when it is empty and keeps `mp3`/`ogg` otherwise.
  - unit tests in `track_meta.rs`:
    - `revision` is equal for two files that differ only in `exports`, whitespace or key order, and
      different when any other field changes (also an unknown one);
    - a non-JSON input hashes the raw bytes;
    - the older-build path: `parse` + `apply_edits` + `to_json_pretty` keeps `exports` verbatim (in
      `extra`).
- done when: `cargo test -p calliope-gui --lib export_record track_meta` passes;
  `cargo clippy -p calliope-gui --all-targets -- -D warnings` is clean.
- routine: no

### Task 2: Repository: `exports` on records, revision, record and clear
- files: `src/repository.rs`, `src/repository_tests.rs`, `src/export_rules.rs` (test literal only),
  `tests/acceptance_tracks_repository.rs` (literal only)
- does:
  - `TrackRecord.exports`;
  - `load` uses `track_meta::revision`;
  - `record_export` and `clear_rc5_holders` (`ClearOutcome`) as in 2.4, with their log lines;
  - update `v1_revision_is_the_hash_of_its_bytes_and_is_migrated_in_memory`, which now expects
    `track_meta::revision(&bytes)`.
  - new tests in `repository_tests.rs`:
    - record each kind (only `exports` differs from before, compared as JSON values; `modified` is
      unchanged);
    - a v1 file stays v1;
    - a problem track is refused and its bytes are unchanged;
    - `clear_rc5_holders` clears every other holder, skips `except` and problem tracks, keeps the
      holders' `mp3`/`ogg` entries, and reports an unwritable holder (`chmod 0555` on the folder,
      skipped as root) in `failed` while the others are still cleared;
    - `save_track` with the revision from *before* `record_export` succeeds and keeps `exports`;
    - a stale revision after a non-export change still conflicts.
- done when: `cargo test -p calliope-gui --lib repository` and
  `cargo test -p calliope-gui --test acceptance_tracks_repository` pass; clippy is clean.
- routine: no

### Task 3: Export rules: memory and start folder
- files: `src/export_rules.rs`
- does:
  - `pub fn memory_for(recorded: Option<u8>, last: Option<u8>) -> u8` (recorded wins, else
    `suggest_memory(last)`);
  - `pub fn file_start_dir(recorded: Option<&Path>, remembered: Option<&Path>, home: Option<&Path>) -> (&'static str, Option<PathBuf>)`
    (2.6);
  - unit tests in the module's `tests`: a recorded file whose folder exists, a missing recorded
    folder falling back to remembered and then to home, and the memory values recorded 7 / none
    with last 98 / none.
- done when: `cargo test -p calliope-gui --lib export_rules` passes; clippy is clean.
- routine: no

### Task 4: Export job: `Finished` / `Recorded` hook, event and snapshot fields
- files: `src/export_job.rs`, `src/export_job_tests.rs`
- does:
  - the `OnDone` signature, `Finished`, `Recorded`, `ExportEvent::Done {.., root, records, info_error}`,
    `ExportSnapshot.info_error`, and `planned()` returning `PlannedRef` (with `repo_root`), as in
    2.5;
  - adjust the test rig's hook (`move |_| { ..; Recorded::default() }`);
  - add a test: the hook gets the target path, kind and memory, and its `Recorded` shows up in the
    `done` event and the snapshot;
  - a cancelled or failed job never calls it.
- done when: `cargo test -p calliope-gui --lib export_job` passes; clippy is clean (task 5 adjusts
  `export_ipc.rs` to compile: do tasks 4 and 5 in one commit if needed).
- routine: no

### Task 5: Export IPC: record after done, info, start folder
- files: `src/export_ipc.rs`, `src/export_ipc_tests.rs`
- does:
  - `ExportInfo.last`, and `memory` via `memory_for`;
  - the MP3/OGG start folder via `file_start_dir` (log `from=recorded`);
  - `record_finished`, and `do_start_export` building the hook (remember the memory, then record
    under `repo.repo_lock()` into `plan.repo_root`);
  - update `memory_is_remembered_after_done`: track 401, exported to 7, now shows 7, and another
    track shows 8 / 99;
  - tests:
    - `do_start_export` records for all three kinds;
    - an RC-5 export clears another holder;
    - an unwritable track folder (made `chmod 0555` after prepare) gives `done` with `info_error`
      and the file in place;
    - the recorded MP3 folder is the dialog's start (`ScriptedPicker::starts()`);
    - `start_uses_the_track_dir_of_the_plan_when_the_root_changed` also checks that the record went
      into the old root.
- done when: `cargo test -p calliope-gui --lib export_ipc export_job` passes; `cargo build -p calliope-gui`
  and clippy are clean.
- routine: no

### Task 6: Headless acceptance tests
- files: `tests/acceptance_export.rs`
- does:
  - **Helpers:**
    - keep `repo_unchanged()` byte-exact, for refusals, cancel, failure and a declined overwrite;
    - add `repo_unchanged_except_exports()`, which compares every file byte for byte except
      `track.json`, compared as JSON values with `exports` removed;
    - switch the existing *successful* export tests to the new helper.
  - **New `ac_info_*` tests**, one per acceptance criterion and requirement (section 7):
    - stale `at` values are written by hand to prove an update;
    - track A's three entries are written into its `track.json` at test time;
    - "info not saved" uses `chmod 0555`, skipped as root;
    - the older-build check runs a fixture `track.json` with `exports` through `track_meta::parse`
      → `apply_edits` → `to_json_pretty`.
- done when: `cargo test -p calliope-gui --test acceptance_export` passes.
- routine: no

### Task 7: Frontend types, helpers and state
- files: `src/ui/lib/ipc.ts`, `src/ui/lib/export-format.ts`, `src/ui/lib/export-format.test.ts`,
  `src/ui/lib/library-state.svelte.ts`, `src/ui/lib/export-state.svelte.ts`, `src/ui/lib/ipc-export.test.ts`
- does:
  - the types, `formatStamp`, `formatDay`, `exportRows`, `lastText`, `mergeTracks`, the `done`
    handling, the Memory box refill rule (`filledFor`, `refillPending`) and the `export last` /
    `export info-error` log lines (2.7);
  - `export-format.test.ts` sets `process.env.TZ = 'Europe/Bucharest'` at the top of the file
    (vitest isolates files) and expects `2026-10-24T18:30:00Z` → `2026-10-24 21:30`, an invalid
    value → `''`, and the rows and "Last:" texts;
  - add `last` / `info_error` to the test literals that need them.
- done when: `npx vitest run src/ui/lib/export-format.test.ts src/ui/lib/ipc-export.test.ts` passes
  and `npm run check` is clean (in `src/calliope-gui`).
- routine: no

### Task 8: Export pane: "Last:" lines, info error, memory default
- files: `src/ui/components/export/ExportPane.svelte`, `src/ui/views/ExportView.test.ts`
- does:
  - the `targetId` effect, the three `export-last-*` paragraphs and the `export-info-error` line
    (2.7);
  - tests:
    - selecting a track whose info has `last.rc5.memory = 7` (with `memory: 7`) shows 7 in the box
      after the box held something else for another track;
    - typed text survives a same-track reload;
    - the "Last:" texts;
    - a `done` event with `records` updates `lib.library` (the tree record has `exports`) and with
      `info_error` shows the alert while "Written to ..." stays;
  - update the `snapshot()` helper and the `info` literal.
- done when: `npx vitest run src/ui/views/ExportView.test.ts` passes; `npm run check` is clean.
- routine: no

### Task 9: Library track pane rows
- files: `src/ui/components/library/TrackPane.svelte`, `src/ui/views/TrackPane.test.ts`
- does:
  - the read-only export rows after "Modified" and the `track exports` log line (2.7);
  - tests:
    - a never-exported fixture shows no `track-export-*` element;
    - a record with rc5 + mp3 shows exactly those two rows with the formatted values;
    - in edit mode the rows are still `readonly` and the save request has no `exports`;
    - `mergeTracks` during edit mode updates the rows and keeps the draft.
- done when: `npx vitest run src/ui/views/TrackPane.test.ts` passes; `npm run check` is clean.
- routine: no

### Task 10: GUI end-to-end tests and visual review
- files: `tests/gui_export_e2e.rs`, `tests/common/mod.rs`
- does:
  - **common:** add `snapshot_except_exports(dir)` in `tests/common/mod.rs`: the `snapshot` map
    with each `track.json` replaced by its JSON minus `exports`, re-serialized. Use it instead of
    `snapshot(..) == before` after successful exports.
  - **`export_rc5_flow`:**
    - After the first export: EX_MONO's `track.json` has `exports.rc5.memory == 7`, and the log has
      `calliope: export recorded id=<EX_MONO> target=rc5 memory=7`.
    - Between the two app runs, write a stale `at`.
    - After the restart: **do not type the memory**. The `export last id=<EX_MONO> memory=7 rc5=7`
      line proves the box default (the shot is renamed to `export-restart-memory-7`). The
      confirmation path ends in `007_1/007_1.WAV` (as today). After Yes, `at` is newer and the
      memory is still 7.
    - Then select EX_PLAIN, type 7 and export (Yes to overwrite). The log has
      `export cleared id=<EX_MONO> memory=7`, EX_MONO's `track.json` has no `rc5`, and EX_PLAIN's
      has 7.
  - **`export_mp3_and_ogg`:**
    - After both exports, the `mp3` / `ogg` paths are the written files.
    - Pressing "MP3 file" again logs `dialog start kind=export-audio from=recorded dir=<D>` (the
      answers file makes that dialog a cancel).
    - Alt+1 and select the track: `track exports id=<id> rows=mp3,ogg`, with the screenshot
      `library-export-rows`.
  - **New test `export_info_keeps_library_edit`** (requirement 8):
    1. In the Library, select EX_PLAIN, press Ctrl+E and type text into the focused field (read
       the field from the `mode=edit` log line or from the pane's focus; it doesn't matter which
       field).
    2. Alt+6, MP3 to `D`.
    3. Alt+1, Ctrl+S: `saved id=<EX_PLAIN>` with no conflict. `track.json` has the edited value and
       the `mp3` entry, and the row log shows `rows=mp3`.
  - **Screenshots:** `export-last-lines` (Export view at 1280x800 with all three "Last:" lines),
    plus one at 1024x640. Look at them: no overlapping text, and the long path wraps inside the
    pane. Keep `layout_overflow <= 14` at 1280x800.
  - Waits use the existing short helpers (`wait_line` with `secs(10)` / `secs(30)` for done). No new
    knobs.
- done when: `npm run build` then `npm run test:gui -- -- export_` (in `src/calliope-gui`) passes
  for `gui_export_e2e` (the full `npm run test:gui` is run by the coordinator), and the screenshots
  in `target/gui-shots/` were reviewed.
- routine: no

### Task 11: Docs
- files: `docs/ui.md`, `src/calliope-gui/README.md`
- does:
  - `docs/ui.md`: a `(gui-export-info)` bullet covering the Library export rows (label, value
    format, local time, only the kinds present, never editable), the Export view's "Last:" lines,
    the info-error line, and the Memory box following the selected track, with the measured
    1280x800 / 1024x640 notes from task 10.
  - README "Export" section: what is remembered per track, re-export behaviour, and the RC-5 rule
    "one track per memory".
- done when: both files mention the export info; `npm test` passes.
- routine: no

## 4. Test strategy

- **Unit (Rust):**
  - `export_record` (read, set, clear) and `track_meta::revision`, including the older-build path
    (task 1);
  - repository record/clear/revision/save interplay with temp repositories (task 2);
  - rules (task 3);
  - the job hook, events and snapshot (task 4);
  - IPC functions with the `ScriptedPicker`, temp settings and the real local `ffmpeg` (task 5).
- **Acceptance (Rust, headless):** `tests/acceptance_export.rs`, copies of
  `tests/fixtures/library-export` in temp dirs, an RC-5 stood in for by a temp folder with
  `ROLAND/`, MP3/OGG temp folders, the `ScriptedPicker` for dialogs (its `starts()` gives the start
  folder). Unwritable metadata = `chmod 0555` on the track folder (skipped when running as root).
  Never a real repository, drive or dialog.
- **Frontend (vitest + jsdom):** helpers (fixed TZ), `ExportView.test.ts`, `TrackPane.test.ts`,
  IPC mocked with `@tauri-apps/api/mocks`.
- **GUI e2e:** `tests/gui_export_e2e.rs` (an existing binary in `scripts/test-gui.sh`): the real app
  on a private display with scripted folder dialogs and capture audio. Results are read from the
  `calliope:` / `calliope-ui:` log lines and from the files on disk. Screenshots (`gui-shot`) for
  the visual check.
- **Whole suite:** in `src/calliope-gui`, run `npm test` (svelte-check, vitest, vite build, `cargo test --workspace`,
  clippy `-D warnings`) and `npm run test:gui` (all GUI binaries, each on its own private display).
  Never the calliope-stems server on :8765, no sudo, no symphonia.

## 5. Deployment

Laptop app only: `npm run build:app` and install as before. Nothing changes on archserver.
`track.json` gains the optional `exports` object (no schema bump), and older builds load and keep
it. `settings.json` is unchanged.

## 6. Manual checks (owner, laptop with the real RC-5)

1. **Record:** export a track with a tempo to the RC-5, memory 7. In the Library, the track shows
   "RC-5 memory 7 (<today> <local time>)".
2. **Re-export:** in the Export view, select the same track. The Memory box shows 7 and "Last:
   memory 7, <today>". Press "to BOSS RC-5" and pick the drive: the confirmation names
   `.../ROLAND/WAVE/007_1/007_1.WAV`. Press Yes. Play memory 7 on the RC-5: it is the reworked
   track.
3. **One track per memory:** export another track to memory 7. The first track's RC-5 row is gone
   in the Library, and the second one shows memory 7.
4. **MP3:** export a track as MP3 to a folder, then press "MP3 file" again. The folder dialog opens
   in that folder, and Yes replaces the file. Do the same for OGG: the Library shows both rows with
   their full paths.
5. **Edit during an export:** in the Library, start editing a track (change the year), switch to
   Export, export that track as MP3, go back and press Save. It saves without "changed on disk",
   and the MP3 row is still there.
6. **Older build (optional):** run a Calliope build from before this feature on the same
   repository. The exported tracks load, and editing and saving one keeps its `exports` (check
   `track.json`).

## 7. Acceptance mapping

| Criterion / requirement | Tasks | Tests |
|---|---|---|
| Never-exported track exported to memory 7 → RC-5 entry 7 + time; Library "RC-5: memory 7 (...)" | 2, 5, 6, 9, 10 | `ac_info_rc5_recorded`; `TrackPane.test.ts` rows; GUI `export_rc5_flow` + `export_mp3_and_ogg` rows log |
| Track with entry 7 selected → memory box shows 7 | 3, 5, 7, 8, 10 | `ac_info_memory_from_record`; `export_rules` `memory_for`; `ExportView.test.ts`; GUI `export_rc5_flow` (`export last ... memory=7`) |
| Same base → confirmation names `.../007_1/007_1.WAV`; after Yes `at` updated, memory 7 | 5, 6, 10 | `ac_info_rc5_reexport_same_memory`; GUI `export_rc5_flow` |
| Entry 7, exported to 12 → entry 12 | 2, 6 | `ac_info_rc5_other_memory` |
| A has 7, B exported to 7 → B 7, A no RC-5 entry/row, A's MP3/OGG and fields unchanged | 2, 5, 6, 9, 10 | `ac_info_rc5_clears_previous_holder`; `repository_tests` clear tests; GUI `export_rc5_flow` (`export cleared`) |
| MP3 to `D` → full path `D/<Band> - <Title>.mp3` in metadata and Library; OGG separate | 2, 5, 6, 9, 10 | `ac_info_file_paths`; `TrackPane.test.ts`; GUI `export_mp3_and_ogg` |
| MP3 entry in `D` → dialog opens in `D` | 3, 5, 6, 10 | `ac_info_mp3_dialog_starts_in_recorded_folder` (also the fallback); GUI `from=recorded` |
| Cancelled, refused, failed → metadata unchanged | 4, 6 | `ac_info_nothing_recorded_on_cancel_refusal_failure` (byte-exact); existing refusal/cancel tests keep `repo_unchanged()` |
| Edited and saved in the Library → info kept, not editable | 1, 2, 9, 10 | `ac_info_kept_by_library_save`; `repository_tests` stale-revision save; `TrackPane.test.ts`; GUI `export_info_keeps_library_edit` |
| Any export → apart from the info, metadata and files unchanged | 2, 6, 10 | `repo_unchanged_except_exports()` in every successful `ac_*` test; `modified` check; GUI `snapshot_except_exports` |
| Never exported → no export rows | 2, 9 | `ac_info_never_exported_has_none`; `TrackPane.test.ts` |
| Req. 2 (earlier entry kept on failure) | 4, 6 | `ac_info_nothing_recorded_...` with a pre-existing entry |
| Req. 7 ("Last:" next to each button) | 7, 8, 10 | `export-format.test.ts`; `ExportView.test.ts`; GUI screenshot `export-last-lines` |
| Req. 8 (edit in progress not disturbed; metadata unwritable → file exported, "info wasn't saved") | 1, 2, 5, 6, 7, 8, 10 | `ac_info_not_saved_is_reported`; `export_ipc_tests`; `ExportView.test.ts` info-error; GUI `export_info_keeps_library_edit` |
| Req. 9 (`export recorded`, `export cleared` in the log) | 2, 10 | GUI `export_rc5_flow` log assertions |
| Constraint: older tracks load; older builds load and keep tracks with info | 1, 2, 6 | `track_meta` older-build test; `ac_info_older_builds_keep_the_info`; fixtures without `exports` load with empty `exports` |

## 8. Open questions

None. These points were decided here; the owner can overrule any of them:
- times are shown in local time (stored in UTC);
- `modified` is not changed by an export;
- the Memory box is refilled whenever another track is selected (with its recorded memory, else
  last used + 1);
- the Export view shows the full path for MP3/OGG;
- the previous RC-5 holder is cleared even when the exported track's own info could not be written,
  because the memory holds the new audio either way;
- the rewritten `track.json` has its keys in alphabetical order until the next Library save
  (cosmetic only).
