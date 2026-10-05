# Plan: track repository and Library view

## 1. Summary

Calliope gets an on-disk **track repository**: a root folder (default `~/.local/share/calliope/`,
set in Settings) with a `tracks/` folder holding one folder per track. Each track folder has a
versioned `track.json` metadata file, the backing-track audio file and its tablature files. The
**Library** view replaces its placeholder with two columns: a searchable band / album / track
tree on the left and a track pane on the right. The pane shows the metadata read-only, can edit
and save it, can export or delete the track, and has a "Tablature Files" sub-panel that adds,
updates, exports and removes tablatures. All file access happens in Rust. The frontend never
sends file-system paths, and the native file dialogs are opened by Rust.

## 2. Design

### 2.1 What is reused

- IPC pattern (architecture.md): thin `#[tauri::command]`s in `src/ipc.rs` over pure modules,
  typed wrappers in `src/ui/lib/ipc.ts`, `Result<T, String>` errors, `calliope-ui:` log lines.
- Settings store (`src/settings.rs`): per-field lenient loading and atomic save. The atomic write
  moves into a shared helper.
- shadcn-svelte components (button, input, label, dialog, card, separator), plus two new ones:
  `textarea` and `alert-dialog`.
- GUI e2e harness from `tests/gui_e2e.rs`: temp XDG dirs, stderr lines, `gui-shot`, `xdotool`,
  `i3-msg`.

### 2.2 Repository on disk (format version 1)

```
<root>/                          default: <data_dir>/calliope  (Linux: $XDG_DATA_HOME/calliope,
│                                 i.e. ~/.local/share/calliope)
├── calliope-repository.json     {"schema_version": 1}   marks the folder as a repository
├── tracks/
│   └── <track-id>/              folder name == track id (not human-readable, req 6)
│       ├── track.json           metadata (below)
│       ├── backing.mp3          main audio; any plain file name, listed in track.json
│       ├── <tab>.gp5 …          tablature files, plain file names, listed in track.json
│       └── stems/               RESERVED for gui-stem-extracting (not created now)
└── trash/                       deleted tracks and removed tablatures (never emptied by Calliope)
    ├── 20261005T140322Z-<id>/                 a deleted track folder, moved here whole
    └── 20261005T140322Z-<id>-tablatures/<name> removed or replaced tablature files
```

- **Track id** (open in the spec; this default stands, see §8 Assumptions):
  Calliope creates ids as UUIDv7 strings (lower-case, hyphenated, e.g.
  `0199b3f2-6c1e-7a3b-9d2e-4f5a6b7c8d9e`): unique, time-ordered, and standard. Any folder name
  matching `^[0-9A-Za-z][0-9A-Za-z_-]{0,63}$` is accepted as an id, so hand-made tracks work.
  The `id` in `track.json` must equal the folder name. The folder layout is flat with no
  sharding: ext4/NTFS/APFS handle thousands of entries well, and a flat folder is easier to
  inspect by hand. Sharding can be added later behind the repository `schema_version`.
- **File names in metadata** are relative to the track folder (req 5). In version 1, `audio`
  and every `tablatures` entry are *plain file names*: no `/` or `\`, not `.` or `..`, no
  leading `.` (hidden and temp files), no control characters, none of `<>:"|?*` (Windows
  portability), at most 255 bytes, and not `track.json`. Stems will use the reserved `stems/`
  subfolder in a later schema version.
- **Temp files**: `track.json.tmp` and `.<name>.part`. Scanning ignores dot-files and `*.tmp`.
  Stale temp files are left alone and never deleted automatically.

### 2.3 `track.json` (schema version 1)

```json
{
  "schema_version": 1,
  "id": "0199b3f2-6c1e-7a3b-9d2e-4f5a6b7c8d9e",
  "band": "Amber Fields",
  "album": "Northern Roads",
  "title": "Slow Burn",
  "composers": ["Ann Example", "Bo Sample"],
  "year": 2019,
  "source_url": "https://example.org/slow-burn",
  "copyright": "© 2019 Amber Fields",
  "audio": "backing.mp3",
  "tablatures": ["slow-burn.gp5", "slow-burn-solo.gp"],
  "imported": "2026-10-05T14:03:22Z",
  "modified": "2026-10-05T14:10:00Z"
}
```

| Field | Type | Rules |
|---|---|---|
| `schema_version` | integer | required. `1` now. Higher: the track is reported as a problem ("written by a newer Calliope") and never rewritten |
| `id` | string | required, id syntax above, equals the folder name, never editable |
| `band`, `album` | string | may be empty (shown as "(no band)" / "(no album)"), ≤ 200 chars, no control chars, trimmed |
| `title` | string | required, non-empty after trimming, ≤ 200 chars |
| `composers` | string array | ≤ 20 entries, each non-empty, ≤ 200 chars |
| `year` | integer or null | 1..=9999 |
| `source_url` | string or null | ≤ 2000 chars. Plain text; never rendered as a link (clicking a link would navigate the webview) |
| `copyright` | string or null | ≤ 500 chars |
| `audio` | string | plain file name |
| `tablatures` | string array | plain file names, unique case-insensitively, ≠ `audio` |
| `imported`, `modified` | string | RFC 3339 UTC with seconds, `Z` suffix, set by Rust only |

- **Versioning and forward compatibility**: `track_meta::parse` reads the JSON as a
  `serde_json::Value`, checks `schema_version`, runs the migration chain
  (`migrate(value, from) -> value`; empty for v1, but the hook and its test exist), then
  deserialises strictly. **Unknown fields are kept** (`#[serde(flatten)] extra: Map`) and
  written back unchanged, so a file touched by a newer minor addition doesn't lose data when an
  older build saves it. Unlike settings, a track with a wrong field type is *not* repaired
  leniently. It is reported as a problem and left untouched, because a lenient load followed by
  a save would destroy data.
- **Revision**: every loaded track carries `revision` = FNV-1a 64-bit hash of the exact
  `track.json` bytes, as 16 hex digits (a string, because JS numbers can't hold 64 bits). Save
  and delete send it back. If the file changed on disk in the meantime (another Calliope
  instance, or a hand edit), the operation fails with a conflict error and nothing is written.
  This is optimistic concurrency: no lock files, no file watcher.
- **Timestamps** are produced by a small hand-written UTC formatter (days-to-civil
  conversion, unit-tested against known dates). No chrono/time dependency. The frontend shows
  them in local time (`YYYY-MM-DD HH:MM`).

### 2.4 Rust modules

| Module | Responsibility | Tauri? |
|---|---|---|
| `src/fsutil.rs` (new) | `write_atomic(path, bytes)` (tmp + fsync + rename + dir fsync; moved here from settings.rs), `copy_no_clobber(src, dest)` (copy to `.<name>.part`, fsync, then `hard_link` to the final name, which fails atomically if it exists, then remove the part; falls back to exists-check + rename on file systems without hard links), `copy_replace(src, dest)` (part + rename, used only when the user confirmed an overwrite in the save dialog), `validate_file_name`, `fnv1a64_hex`, `create_unique_dir(parent, base)` (`base`, `base (2)`, …), `move_track_to_trash(root, src, stamp, id)` (the track folder itself is renamed to `trash/<stamp>-<id>/`, no nesting; `-2`.. on clash), `TablatureTrash::new(root, stamp, id)` + `move_in(src)` (one `trash/<stamp>-<id>-tablatures/` per save, created on first use; a file name clash gets a ` (2)` suffix on the file, not a new folder); both refuse the root and anything already under `trash/` | no |
| `src/track_meta.rs` (new) | `TrackMeta` struct + `extra`, `parse(bytes, dir_name) -> Result<TrackMeta, String>`, `to_json`, `validate`, `TrackEdits` + `apply_edits`, `is_valid_id`, `new_id()` (uuid v7), `now_rfc3339()` / `rfc3339_utc(secs)` | no |
| `src/repository.rs` (new) + `src/repository_tests.rs` | `Repository { root }`: `status(root) -> RepoStatus`, `ensure_layout()` (marker + `tracks/`, only adds), `scan() -> Library { tracks, problems }`, `create_track(NewTrack, audio_src)` (for tests, the fixture builder and the future import feature), `save_track(SaveTrackRequest, &PickRegistry) -> SaveResult`, `delete_track(id, revision)`, `export_track(id, dest_parent) -> PathBuf`, `export_tablature(id, name, dest)`. Every id and name from IPC is validated and resolved strictly under `<root>/tracks/<id>` | no |
| `src/picker.rs` (new) | `trait Picker { pick_file(&FileReq) / pick_folder(&FolderReq) / save_file(&SaveReq) -> Option<PathBuf> }`; `PickRegistry` (token → (kind, path), tokens `p<counter>`, kept until used or the app exits); `TauriPicker` (tauri-plugin-dialog, blocking calls, run via `spawn_blocking`); `ScriptedPicker`, compiled only with the `e2e-hooks` feature | `TauriPicker` only |
| `src/settings.rs` (extend) | new field `repository_root: Option<PathBuf>` (absolute; anything else is a warning and the default), `set_repository_root(Option<PathBuf>)`, `default_repository_root(data_dir) -> PathBuf` (pure) | no |
| `src/ipc.rs` (extend) | new commands (below), `async`, IO in `spawn_blocking` | yes |
| `src/gui.rs` (extend) | registers `tauri_plugin_dialog`, manages `RepoState` (Mutex over the current root, plus the `PickRegistry`) and the `Picker` | yes |

`RepoState` holds one `Mutex`, and every repository operation runs under it, so two commands
never interleave file operations within one app instance.

**Save transaction** (`save_track`): data safety comes first. The order is:
1. Lock. Resolve the track folder, re-read `track.json`, compare `revision` (conflict: abort).
2. Validate the edits, every tab entry (`keep` names must exist in the current metadata;
   `add`/`replace` tokens must be `tablature` picks in the registry; resulting names are unique
   case-insensitively, valid file names, ≠ `audio`, ≠ `track.json`) and every source file
   (exists, is a regular file).
3. Copy new files in: `add` uses `copy_no_clobber` (a name clash with an untracked file
   already in the folder is an error, never an overwrite). For a `replace` with a *different*
   name, also no-clobber. For a `replace` with the *same* name, copy to `.<name>.part` and keep
   it there for step 5.
4. Build the new metadata (`modified = now`, `imported` kept, `extra` kept) and `write_atomic`
   `track.json`. **If this fails**, delete only the files created in step 3 and return the
   error. The folder is then exactly as before.
5. Move removed and replaced old files to `trash/<stamp>-<id>-tablatures/`. For a same-name
   replace: move the old file to the trash, then rename the `.part` file into place. Failures
   here don't undo the save. They are returned as `warnings` (shown in the pane) and logged.
6. Return the re-scanned record with its new revision.

**Delete track**: lock, check the revision, `rename` `tracks/<id>` → `trash/<stamp>-<id>/` (one
atomic rename on the same file system). Nothing is ever removed with `remove_file`/`remove_dir`
except Calliope's own temp/part files.

**Export track**: Rust opens a folder dialog ("Export track to…"). In the chosen folder it
creates a new folder `"<band> - <album> - <title>"` (characters invalid in file names are
replaced by `_`, empty parts are dropped, `(2)`, `(3)` … are added if the name exists). It copies
`track.json`, the audio and all tablatures into it (no-clobber) and returns the new path. A
destination inside `<root>/tracks` is refused. Missing source files make the export fail
before anything is copied.

**Export tablature**: Rust opens a save dialog ("Export tablature", default name = the
tablature name, tablature filter). The GTK dialog asks the user to confirm overwriting (rfd's
gtk3 backend sets `do_overwrite_confirmation`; verified in rfd 0.15 source, re-check in 0.16),
so the copy uses `copy_replace`.

**Scan**: lists `tracks/*` folders (skipping dot-names). It reads each `track.json`. Problems (a
missing or invalid file, an id mismatch, a newer schema, an invalid folder name) go into
`problems: [{dir, message}]` and never stop the scan. Each record lists `missing`: metadata
files that aren't on disk. The scan never writes anything, except that when the root is the
**default** root, `list_tracks` first runs `ensure_layout()` (so a first start just works). A
*configured* root that doesn't exist is reported as status `missing` and is not created; the
Library tells the user to check Settings (for example, an unmounted USB disk must not get a new
empty repository written to the mount point).

### 2.5 File dialogs and test injection

- All dialogs are opened **from Rust** via `tauri-plugin-dialog` 2.x (`DialogExt`,
  `blocking_pick_file/pick_folder/save_file`, inside `async` commands with `spawn_blocking`,
  never on the main thread). The JS side of the plugin is **not** used and gets no permission.
  The frontend therefore never handles absolute paths it could forge: a pick returns an opaque
  `token` plus the bare file name, and `save_track` / `set_repository_root` accept tokens only.
  A compromised or buggy webview can't make Calliope read `~/.ssh/id_rsa` or write outside
  the repository.
- Dialogs: "Add tablature" / "Update tablature" (open file, parent = main window, start
  directory = the user's home, filter "Tablature files": `gp gp3 gp4 gp5 gpx tg ptb musicxml mxl xml`,
  no "all files" entry; Rust also checks the extension), "Choose track repository folder"
  (folder), "Export track to…" (folder), "Export tablature" (save file).
- Every dialog logs `calliope: dialog kind=<add-tablature|update-tablature|repository-root|export-track|export-tablature> result=<picked|cancelled>`
  to stderr. The e2e tests wait on these lines.
- **Test injection**: cargo feature `e2e-hooks` (off by default). Only when the binary is
  built with it **and** `CALLIOPE_E2E_DIALOG_ANSWERS=<file>` is set, `gui.rs` installs
  `ScriptedPicker`. It pops the first line of that file per dialog: `<kind> <absolute path>`
  or `<kind> CANCEL`. A kind mismatch or an empty file means a cancel plus a
  `calliope: e2e dialog script mismatch …` line. It cannot be reached in release builds:
  `build.rs` fails a `release` profile build that has `CARGO_FEATURE_E2E_HOOKS` set, and a
  static test checks that `build:app`/`app` in `package.json` never mention the feature. Only
  `npm run test:gui` passes `--features e2e-hooks`. Without the env var, even an e2e-hooks
  binary uses the real dialogs. One e2e test relies on that to check the real GTK dialog.

### 2.6 Security: ACL / capabilities

The repository commands reach the file system, so the app now declares an **app ACL
manifest**, as the reviewer asked. `build.rs` calls
`tauri_build::try_build(Attributes::new().app_manifest(AppManifest::new().commands(&[...all app commands...])))`,
which autogenerates `allow-<command>` permissions into `permissions/autogenerated/`
(gitignored, regenerated on each build). `capabilities/main.json` grants them to window
`main`, local origin only:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "main",
  "description": "Calliope's own commands for the main window. No plugin or core JS APIs.",
  "local": true,
  "windows": ["main"],
  "permissions": [
    "allow-app-version", "allow-get-settings", "allow-set-theme", "allow-frontend-log",
    "allow-get-repository", "allow-choose-repository-root", "allow-set-repository-root",
    "allow-reset-repository-root", "allow-list-tracks", "allow-pick-tablature",
    "allow-save-track", "allow-delete-track", "allow-export-track", "allow-export-tablature"
  ]
}
```

No `core:*`, `dialog:*` or `fs:*` permission is granted. If the e2e run shows that the webview
needs a core permission, add exactly that one and record why in architecture.md. A static
test keeps three lists equal: `generate_handler![...]`, the `build.rs` command list, and the
capability's `allow-*` set. The CSP is unchanged.

### 2.7 IPC commands (new)

| Command | Args | Returns | Notes |
|---|---|---|---|
| `get_repository` | – | `RepoInfo {root, is_default, status}` | status: `ok` (marker present), `empty` (exists, empty), `other` (exists, non-empty, no marker), `missing`, `newer` (marker schema > 1) |
| `choose_repository_root` | – | `PickedRoot {token, root, status} \| null` | folder dialog. The setting is **not** changed yet |
| `set_repository_root` | `token` | `RepoInfo` | refuses `newer`; runs `ensure_layout` (adds the marker and `tracks/` only), saves the setting |
| `reset_repository_root` | – | `RepoInfo` | clears the setting (default root) |
| `list_tracks` | – | `Library {root, tracks: TrackRecord[], problems}` | `TrackRecord` = metadata fields (without `schema_version`/extra) + `revision` + `missing` |
| `pick_tablature` | `purpose: "add" \| "update"` | `Picked {token, name} \| null` | errors on a bad name/extension |
| `save_track` | `request: SaveTrackRequest {id, revision, edits, tablatures: TabEntry[]}` | `SaveResult {track, warnings}` | `TabEntry` = `{kind:"keep",name}` \| `{kind:"add",token}` \| `{kind:"replace",name,token}`; the list order is the new order; existing names that are left out are removed |
| `delete_track` | `id, revision` | `()` | moves the folder to the trash |
| `export_track` | `id` | `string \| null` | folder dialog; returns the created folder |
| `export_tablature` | `id, name` | `string \| null` | save dialog |

`get_settings` now also returns `repository_root: string | null`. All argument names are single
words, so Tauri's camelCase/snake_case argument mapping can't bite.

### 2.8 Frontend

```
src/ui/lib/ipc.ts                 + repository types and wrappers (Task 8, given verbatim)
src/ui/lib/fuzzy.ts               normalize(), tokenMatches(), trackMatches()
src/ui/lib/library-tree.ts        buildTree(tracks), filterTree(tree, query), node keys, collator sort
src/ui/lib/track-draft.ts         Draft model: from record, field edits, tab staging, validation,
                                  toSaveRequest(), buttonStates(mode, selection, draft)
src/ui/lib/library-state.svelte.ts module-level $state: library, loading/error, expanded keys,
                                  search, selectedId, mode ('view'|'edit'), draft, busy, message
src/ui/components/library/TreeToolbar.svelte   collapse all, expand all, search box, clear
src/ui/components/library/TrackTree.svelte     ARIA tree, roving tabindex
src/ui/components/library/TrackPane.svelte     fields + bottom action bar
src/ui/components/library/TablaturePanel.svelte list + Add/Update/Export/Remove
src/ui/components/ConfirmDialog.svelte          Yes/No alert dialog (reusable)
src/ui/components/RepositorySettings.svelte     Settings card
src/ui/views/LibraryView.svelte                 two columns
```

- **Tree** (req 9, AC1-2): three levels (band → album → track title). Grouping is
  case-insensitive on the trimmed name; the shown label is the first spelling in sort order.
  Each level is sorted with `Intl.Collator(undefined, {sensitivity: 'base', numeric: true})`,
  ties broken by the exact string and then the id. "(no band)" / "(no album)" sort last.
  Initially every band is collapsed. Collapse-all/expand-all affect all band and album nodes.
  Keyboard: ↑/↓ move, → expands, ← collapses or goes to the parent, Enter/Space selects a
  track or toggles a group, Home/End. Node keys are `b:<band>` and `b:<band>|a:<album>`
  (normalized), so the expansion state survives a rescan.
- **Search** (req 12, AC5): filters on every keystroke. Case-insensitive and
  diacritic-insensitive (NFD, strip `\p{M}`, lower-case). The query is split on whitespace,
  and **every** token must match **one** of band/album/title. A token matches a field if it is
  a substring of the field, *or* a subsequence of the field whose first character is at a word
  start. Examples on the fixture: `nght` → "the Night Owls"; `brn` → "Slow Burn"; `ber` →
  "Amber Fields" (substring); `mbr` → nothing; `owls lan` → "Lanterns"; `cafe` → "Café". While
  the query is non-empty, every branch with a match is shown expanded. Clearing (button or
  emptying the box) restores the expansion state from before the search. No matches shows
  `No tracks match "<query>"`. Our own small matcher: no library, no copied code.
- **Pane** (req 10-11, AC6-10): fields in a labelled grid. Editable: Band, Album, Title,
  Composers (textarea, one per line), Year (number, blank = none), Source link, Copyright.
  Always read-only: Track ID, Audio file, Imported, Modified. With no selection or an empty
  library, all fields are shown empty and disabled, the action bar is visible with all buttons
  disabled, and the Tablature panel is disabled. The **action bar** is pinned to the bottom of
  the pane (the fields scroll above it): `Edit`, `Save`, `Cancel`, `Export`, `Delete`.
  - View mode with a selection: Edit, Export, Delete enabled; Save and Cancel disabled.
  - Edit mode: Save and Cancel enabled; Edit, Export, Delete disabled; Tablature panel enabled;
    the tree and search are disabled (`inert`), so the selection can't change mid-edit.
  - Save: validation errors show next to the fields and the pane stays in edit mode; success
    → view mode, values from the returned record (new `modified`), and any warnings shown
    inline. Save is enabled in edit mode even without changes, and still writes (AC: modified =
    save time).
  - Cancel / Escape: if the draft changed, confirm "Discard your changes?" (Yes/No), then go back
    to view mode. (Cancel: owner decision, §10.)
  - Delete: confirm `Delete track "<title>"? It will be moved to the repository's trash folder.`
    Yes/No (focus starts on No).
  - Export: dialog in Rust; on success, inline `Exported to <path>`.
  - Messages and errors are shown inline in the pane (`role="status"` / `role="alert"`), never
    as toasts or pop-ups (docs/ui.md).
  - The draft lives in `library-state`, so switching views with Alt+N and back keeps an
    unfinished edit. Changing the repository root in Settings clears the library state.
- **Tablature panel** (req 13, AC11-19): label "Tablature Files"; a `listbox` whose height fits
  4 rows (overflow-y auto, scrollbar when more). Buttons `Add`, `Update`, `Export`, `Remove`.
  Each row shows the file name and a marker: "new" for a staged add, "updated" for a staged
  replace, and "missing" for a file that is in the metadata but not on disk.
  - Disabled unless in edit mode. In edit mode with no selection: only Add is enabled. With a
    selection: Update, Remove, and Export (Export is disabled for staged "new"/"updated" rows,
    which aren't in the repository yet, and for "missing" rows).
  - Add → `pick_tablature('add')`. Cancel: no change. Picked: if the name clashes
    (case-insensitively) with a listed one, or equals the audio name or `track.json`, show an
    inline error; otherwise append it as "new", select it, and Remove becomes enabled.
  - Update → `pick_tablature('update')` → the selected row is replaced by the picked name
    (marked "updated"). The old file goes to the trash on Save.
  - Remove → confirm `Delete tablature "<name>"?` (owner decision, §10), Yes →
    the row leaves the list (the metadata is untouched until Save), and the selection is
    cleared.
  - Export → `export_tablature(id, name)`.
- **Keyboard (Library)**: `Ctrl+F` focuses the search box, ↓ in the search box moves to the
  first visible tree item, `Ctrl+E` = Edit, `Ctrl+S` = Save, `Escape` = Cancel (edit mode, no
  dialog open). They are handled only while the Library view is active. The e2e tests use them,
  and so can the user.
- **Settings card** "Track repository" (second, after Appearance): the effective path
  (selectable text) with "(default)" when unset, a status line for `missing`/`newer`, and the
  buttons `Choose folder…` and `Use default`. Choose → `choose_repository_root()`. If the status
  is `other`, a confirmation follows: `Use "<path>" as the track repository? Calliope will add a
  "tracks" folder and a "calliope-repository.json" file there. Existing files are not
  changed.` `newer` shows an error. Then `set_repository_root(token)`. Changing the root never
  moves or copies tracks.
- **Log lines** (frontend, via `frontend_log`): `library root=<path> status=<s> tracks=<n> problems=<m>`,
  `select id=<id>`, `mode=edit id=<id>`, `mode=view id=<id>`, `saved id=<id> tablatures=<n> warnings=<w>`,
  `deleted id=<id>`, `exported id=<id>`, `tab-staged <add|update|remove> name=<name>`,
  `repository root=<path> default=<bool>`, `error <context>: <msg>`.
- **Problems** (unreadable track folders): a line under the tree, "2 track folders could not be
  read", which expands into a list of `<dir>: <message>`.
- **Layout**: the left column is `w-80` (20 rem, about 360 px at the 18 px root size) with its
  own scroll; the pane takes the rest. At 1024x640 with the nav expanded, the pane is about
  420 px wide and its field area scrolls above the pinned action bar (checked visually).

### 2.9 Dependencies added

| Dependency | Kind | Licence | Why |
|---|---|---|---|
| `tauri-plugin-dialog` 2 (pulls `tauri-plugin-fs`, `rfd`) | Rust | MIT OR Apache-2.0 | native file/folder/save dialogs from Rust |
| `uuid` 1 (`v7` feature) | Rust | MIT OR Apache-2.0 | track ids (already in Cargo.lock via tauri) |
| shadcn-svelte `textarea`, `alert-dialog` (copied source) | npm (bits-ui already present) | MIT | composers field, confirmations |

The overview asks for a record of dependency licences. `docs/licences.md` is created (Task 17)
for all direct dependencies, and a static test keeps it complete.

## 3. Tasks

All commands run from `/home/vali/src/calliope`. The headless suite is `npm test`; GUI tests
are `DISPLAY=:1 npm run test:gui`. **No task may read or write `~/.local/share/calliope`,
`~/.config/app.calliope.gui` or any real user folder**: Rust tests use `tempfile`, frontend
tests use `mockIPC`, and GUI tests use temp XDG dirs under `target/gui-e2e/`.

### Task 1: `fsutil` module: atomic write, safe copy, name validation, trash
- **files**: `src/fsutil.rs` (new), `src/settings.rs`, `src/main.rs`
- **does**: Implement the `fsutil` functions from §2.4. Move the atomic-write code out of
  `settings::save` into `fsutil::write_atomic(path, bytes)` (same behaviour: `<path>.tmp`,
  fsync, rename, best-effort dir fsync) and make `settings::save` call it.
  `validate_file_name(&str) -> Result<(), String>` implements the rules of §2.2.
  `copy_no_clobber` uses part file + `hard_link` (falls back to an exists check + rename when
  `hard_link` returns `Unsupported`/`PermissionDenied`), and removes the part file on any error.
  `move_track_to_trash(root, src, stamp, id)` renames the track folder itself to
  `trash/<stamp>-<id>/` (suffix `-2`… if needed). `TablatureTrash::new(root, stamp, id)` with
  `move_in(&mut self, src)` moves each removed/replaced file into one
  `trash/<stamp>-<id>-tablatures/` per save (created on first use; a clash inside it suffixes
  the file as `name (2).ext`). Both refuse (error) if `src` isn't strictly inside `root` or is
  already under `trash/`; paths are canonicalised. `fnv1a64_hex(bytes)`. `create_unique_dir`.
  Unit tests for each, on `tempfile` dirs: no-clobber refuses an existing dest and leaves no
  part file; `write_atomic` leaves no tmp; names `..`, `a/b`, `.x`, `track.json`, `a\b`, `x?`,
  `""` and 256-byte names are rejected and `Slow Burn (live).gp5` is accepted; FNV of `""` is
  `cbf29ce484222325` and of `"a"` is `af63dc4c8601ec8c`; trash moves keep the content and
  never remove anything.
- **done when**: `cargo test fsutil` and `cargo test settings` pass, and `cargo clippy --all-targets -- -D warnings` is clean.
- **test**: `cargo test && cargo clippy --all-targets -- -D warnings`
- **routine: no** (data-safety code)

### Task 2: `track_meta` module: schema v1, validation, timestamps, ids
- **files**: `src/track_meta.rs` (new), `src/main.rs`, `Cargo.toml` (`uuid = { version = "1", features = ["v7"] }`)
- **does**: `TrackMeta` per §2.3 with `#[serde(flatten)] extra: serde_json::Map`;
  `CURRENT_SCHEMA = 1`; `parse(bytes, dir_name)` (Value → schema check → `migrate` → strict
  deserialize → `validate` → id == dir_name), with errors as readable strings
  (`missing schema_version`, `written by a newer Calliope (schema 2)`, `id "x" does not match folder "y"`, …);
  `to_json_pretty`; `TrackEdits` + `apply_edits(&mut meta, edits)` (trims, turns empty
  optional strings into `None`, validates); `is_valid_id`; `new_id()`;
  `rfc3339_utc(unix_secs) -> String`; `now_rfc3339()`. Unit tests: round trip of the §2.3
  example; unknown fields survive parse → to_json; schema 2 and a missing schema are errors; a
  wrong type (`"year": "1999"`) is an error; title "  " is rejected; 21 composers are rejected;
  `rfc3339_utc(0) == "1970-01-01T00:00:00Z"`,
  `rfc3339_utc(951782400) == "2000-02-29T00:00:00Z"`,
  `rfc3339_utc(1790000000) == "2026-09-21T14:13:20Z"`; `new_id()` is valid and two calls differ.
- **done when**: `cargo test track_meta` passes; clippy is clean.
- **test**: `cargo test && cargo clippy --all-targets -- -D warnings`
- **routine: no**

### Task 3: `repository` module: layout, scan, save transaction, delete, export
- **files**: `src/repository.rs` (new), `src/repository_tests.rs` (new; included with
  `#[cfg(test)] #[path = "repository_tests.rs"] mod tests;`), `src/main.rs`
- **does**: Everything in §2.4 "Repository": `RepoStatus`, `status`, `ensure_layout`, `scan`,
  `create_track`, `save_track` (exact step order of the save transaction), `delete_track`,
  `export_track`, `export_tablature`. `save_track` takes a `&PickRegistry`-like lookup as a
  trait or closure (`Fn(&str) -> Option<PathBuf>`), so this module doesn't depend on
  `picker.rs`. Tests (temp repositories, built with `create_track`, each under a
  `tempdir()/repo` plus a sibling `tempdir()/outside` whose listing must be unchanged at the
  end of every test):
  scan finds tracks and reports bad JSON / id mismatch / newer schema / invalid folder names as
  problems without failing; `missing` lists absent files; saving edits updates `modified` and
  keeps `imported` and `extra`; add copies the file and lists it; remove moves the file to
  `trash/` and drops it from the metadata; same-name replace leaves the new content in place
  and the old one in the trash; a revision conflict changes nothing; a bad token, a `keep` of an
  unknown name, ids like `../x`, `a/b` or `.hidden`, and tab names like `../../x` are rejected
  with nothing written; an untracked file with the same name as an added tab is never
  overwritten; a simulated metadata-write failure (read-only track folder, `#[cfg(unix)]`)
  leaves no new files behind; delete moves the whole folder to the trash; export creates
  `"Band - Album - Title"` and then `"… (2)"`, and refuses a destination inside `tracks/`;
  `ensure_layout` in a non-empty foreign folder adds only the marker and `tracks/`.
- **done when**: `cargo test repository` passes (at least 20 tests); clippy is clean.
- **test**: `cargo test && cargo clippy --all-targets -- -D warnings`
- **routine: no** (core data-safety logic)

### Task 4: Sample repository fixture
- **files**: `tests/fixtures/library-sample/**` (new), `src/repository_tests.rs` (one test)
- **does**: A committed sample repository: `calliope-repository.json` and 6 tracks with fixed
  UUIDv7-shaped ids `0199b0a0-0000-7000-8000-00000000000N` (N = 1..6), each with
  `backing.mp3` (content: `calliope test fixture: not real audio\n`) and the tablature files
  listed (content: `calliope test fixture: not a real tablature\n`). Names are invented (no real
  artists):

  | N | band | album | title | composers | year | tablatures |
  |---|---|---|---|---|---|---|
  | 1 | Amber Fields | Northern Roads | Slow Burn | Ann Example, Bo Sample | 2019 | slow-burn.gp5, slow-burn-solo.gp |
  | 2 | Amber Fields | Northern Roads | after midnight | Ann Example | 2019 | – |
  | 3 | Amber Fields | Copper Sky | Copper Sky | Bo Sample | 2021 | copper-sky.gp5 |
  | 4 | the Night Owls | Lanterns | Lanterns | Cy Placeholder | null | – |
  | 5 | Zephyr Lane | (empty) | Open Water | – | 2024 | open-water-rhythm.gp5, open-water-lead.gp5, open-water-bass.gp5, open-water-intro.gp, open-water-full.gpx |
  | 6 | (empty) | (empty) | Café Practice Groove | – | null | – |

  Track 1 also has `source_url` `https://example.org/slow-burn` and copyright
  `© 2019 Amber Fields (fixture)`; the others have null. `imported` = `modified` =
  `2026-10-01T12:00:00Z`. Add the test `fixture_library_sample_scans_clean`: scanning a copy of
  the fixture gives 6 tracks, 0 problems, 0 missing.
- **done when**: that test passes; `git status` shows only files under `tests/fixtures/library-sample/` and the test file.
- **test**: `cargo test fixture_library_sample`
- **routine: no** (about 18 files with exact content; too long for the local model)

### Task 5: Settings: repository root
- **files**: `src/settings.rs`
- **does**: Add `repository_root: Option<PathBuf>` (JSON `"repository_root": "/abs"`; absent or
  `null` = default). Lenient per field like `theme`: a non-string or relative path logs a
  warning and becomes `None`, and the other fields are kept. `SettingsStore::set_repository_root(Option<PathBuf>) -> io::Result<Settings>`.
  `pub fn default_repository_root(data_dir: &Path) -> PathBuf { data_dir.join("calliope") }`.
  Update `json_shape` to the new shape (`{"theme":"light","repository_root":null}`) and add
  tests: relative path → default + theme kept; round trip of an absolute path; set/reset persists.
- **done when**: `cargo test settings` passes; clippy is clean.
- **test**: `cargo test && cargo clippy --all-targets -- -D warnings`
- **routine: no** (small, but not given verbatim)

### Task 6: Picker: dialog plugin, token registry, scripted e2e picker, release guard
- **files**: `src/picker.rs` (new), `src/main.rs`, `Cargo.toml` (`tauri-plugin-dialog = "2"`,
  `[features] e2e-hooks = []`), `build.rs`, `tests/frontend.rs`
- **does**: As §2.5: `Picker` trait and request structs (title, filters, default name);
  `PickRegistry` (`insert(kind, path) -> token`, `get(kind, token)`, `take`); `TauriPicker`
  (plugin `DialogExt`, `set_parent(main window)`, logs the `calliope: dialog kind=… result=…`
  line); `#[cfg(feature = "e2e-hooks")] ScriptedPicker` reading `CALLIOPE_E2E_DIALOG_ANSWERS`
  (pops the first line and rewrites the file; logs the same `dialog` line); tablature
  extension check `is_tablature_name`. `build.rs`: if `PROFILE == release` and
  `CARGO_FEATURE_E2E_HOOKS` is set → `fail("the e2e-hooks feature must never be in a release build")`.
  Static test in `tests/frontend.rs`: the `app`, `build:app` and `build` scripts in
  `package.json` don't contain `e2e-hooks`, and `default` features in `Cargo.toml` don't
  include it. Unit tests for the registry (a token of another kind is rejected, an unknown
  token is `None`) and for `ScriptedPicker` (pops in order, mismatch → `None`, missing file →
  `None`), using a temp answers file.
- **done when**: `cargo test` and `cargo test --features e2e-hooks picker` pass;
  `cargo build --release --features e2e-hooks` **fails** with the message (check it, then
  don't keep the artifact); clippy is clean with and without the feature.
- **test**: `cargo test && cargo test --features e2e-hooks picker && cargo clippy --all-targets -- -D warnings && cargo clippy --all-targets --features e2e-hooks -- -D warnings`
- **routine: no** (security boundary)

### Task 7: IPC commands, app state, ACL manifest and capability
- **files**: `src/ipc.rs`, `src/gui.rs`, `build.rs`, `capabilities/main.json` (new; content in
  §2.6), `.gitignore` (`permissions/autogenerated/`), `tests/frontend.rs`
- **does**: The commands of §2.7, as `async` commands whose file IO and dialogs run in
  `tauri::async_runtime::spawn_blocking`, with errors mapped to readable strings. `RepoState`
  in `gui.rs` setup: root = `settings.repository_root` or
  `default_repository_root(app.path().data_dir()?)`. `.plugin(tauri_plugin_dialog::init())`. The
  picker is `ScriptedPicker` only under `cfg(feature = "e2e-hooks")` with the env var set,
  else `TauriPicker`. Register all commands. In `build.rs` replace `tauri_build::build()` with
  `try_build(Attributes::new().app_manifest(AppManifest::new().commands(&COMMANDS)))` (fail
  with the error). Static test: the names in `generate_handler!` (parsed from `src/gui.rs`), the
  `COMMANDS` list in `build.rs`, and the `allow-*` entries in `capabilities/main.json` (with
  `_` → `-`) are the same set; the capability has `"windows": ["main"]`, `"local": true` and
  no `core:`/`dialog:`/`fs:` entries. Check that a second `cargo build` is a no-op (no
  rebuild loop from `permissions/`). Then run the existing GUI tests to prove that the
  existing commands still pass the ACL.
- **done when**: `npm test` passes; `cargo build && cargo build 2>&1 | grep -c Compiling` prints `0`;
  `DISPLAY=:1 npm run test:gui` passes (existing smoke/e2e: ready line, theme switch, no
  "not allowed" errors on stderr).
- **test**: `npm test && DISPLAY=:1 npm run test:gui`
- **routine: no** (ACL/security, cross-cutting)

### Task 8: Frontend IPC types and wrappers
- **files**: `src/ui/lib/ipc.ts` (replace the whole file), `src/ui/lib/ipc-repository.test.ts` (new)
- **does**: Write both files with exactly this content.

  `src/ui/lib/ipc.ts`:
  ```ts
  import { invoke } from '@tauri-apps/api/core';

  export type Theme = 'dark' | 'light';
  export interface Settings { theme: Theme; repository_root: string | null }

  export const appVersion = (): Promise<string> => invoke<string>('app_version');
  export const getSettings = (): Promise<Settings> => invoke<Settings>('get_settings');
  export const setTheme = (theme: Theme): Promise<Settings> => invoke<Settings>('set_theme', { theme });
  export const frontendLog = (message: string): Promise<void> =>
    invoke<void>('frontend_log', { message }).catch(() => undefined);

  // Track repository: mirrors src/track_meta.rs, src/repository.rs and src/ipc.rs.
  export interface TrackRecord {
    id: string;
    band: string;
    album: string;
    title: string;
    composers: string[];
    year: number | null;
    source_url: string | null;
    copyright: string | null;
    audio: string;
    tablatures: string[];
    imported: string;
    modified: string;
    revision: string;
    missing: string[];
  }
  export interface LibraryProblem { dir: string; message: string }
  export interface Library { root: string; tracks: TrackRecord[]; problems: LibraryProblem[] }
  export type RepoStatus = 'ok' | 'empty' | 'other' | 'missing' | 'newer';
  export interface RepoInfo { root: string; is_default: boolean; status: RepoStatus }
  export interface PickedRoot { token: string; root: string; status: RepoStatus }
  export interface Picked { token: string; name: string }
  export type PickPurpose = 'add' | 'update';
  export interface TrackEdits {
    band: string;
    album: string;
    title: string;
    composers: string[];
    year: number | null;
    source_url: string | null;
    copyright: string | null;
  }
  export type TabEntry =
    | { kind: 'keep'; name: string }
    | { kind: 'add'; token: string }
    | { kind: 'replace'; name: string; token: string };
  export interface SaveTrackRequest { id: string; revision: string; edits: TrackEdits; tablatures: TabEntry[] }
  export interface SaveResult { track: TrackRecord; warnings: string[] }

  export const getRepository = (): Promise<RepoInfo> => invoke<RepoInfo>('get_repository');
  export const chooseRepositoryRoot = (): Promise<PickedRoot | null> =>
    invoke<PickedRoot | null>('choose_repository_root');
  export const setRepositoryRoot = (token: string): Promise<RepoInfo> =>
    invoke<RepoInfo>('set_repository_root', { token });
  export const resetRepositoryRoot = (): Promise<RepoInfo> => invoke<RepoInfo>('reset_repository_root');
  export const listTracks = (): Promise<Library> => invoke<Library>('list_tracks');
  export const pickTablature = (purpose: PickPurpose): Promise<Picked | null> =>
    invoke<Picked | null>('pick_tablature', { purpose });
  export const saveTrack = (request: SaveTrackRequest): Promise<SaveResult> =>
    invoke<SaveResult>('save_track', { request });
  export const deleteTrack = (id: string, revision: string): Promise<void> =>
    invoke<void>('delete_track', { id, revision });
  export const exportTrack = (id: string): Promise<string | null> =>
    invoke<string | null>('export_track', { id });
  export const exportTablature = (id: string, name: string): Promise<string | null> =>
    invoke<string | null>('export_tablature', { id, name });
  ```

  `src/ui/lib/ipc-repository.test.ts`:
  ```ts
  import { afterEach, describe, expect, it } from 'vitest';
  import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
  import {
    chooseRepositoryRoot, deleteTrack, exportTablature, exportTrack, getRepository, listTracks,
    pickTablature, resetRepositoryRoot, saveTrack, setRepositoryRoot, type SaveTrackRequest,
  } from './ipc';

  afterEach(() => clearMocks());

  describe('repository ipc wrappers', () => {
    it('send the command names and arguments', async () => {
      const calls: [string, unknown][] = [];
      mockIPC((cmd, args) => { calls.push([cmd, args]); return null; });
      const req: SaveTrackRequest = {
        id: 'abc',
        revision: '00ff',
        edits: { band: 'B', album: 'A', title: 'T', composers: ['C'], year: 1999, source_url: null, copyright: null },
        tablatures: [{ kind: 'keep', name: 'x.gp5' }, { kind: 'add', token: 'p1' }],
      };
      await getRepository();
      await chooseRepositoryRoot();
      await setRepositoryRoot('p2');
      await resetRepositoryRoot();
      await listTracks();
      await pickTablature('add');
      await saveTrack(req);
      await deleteTrack('abc', '00ff');
      await exportTrack('abc');
      await exportTablature('abc', 'x.gp5');
      expect(calls).toEqual([
        ['get_repository', {}],
        ['choose_repository_root', {}],
        ['set_repository_root', { token: 'p2' }],
        ['reset_repository_root', {}],
        ['list_tracks', {}],
        ['pick_tablature', { purpose: 'add' }],
        ['save_track', { request: req }],
        ['delete_track', { id: 'abc', revision: '00ff' }],
        ['export_track', { id: 'abc' }],
        ['export_tablature', { id: 'abc', name: 'x.gp5' }],
      ]);
    });

    it('returns null when a dialog is cancelled', async () => {
      mockIPC(() => null);
      expect(await pickTablature('update')).toBeNull();
      expect(await exportTrack('abc')).toBeNull();
    });
  });
  ```
- **done when**: `npm run check` and `npm run test:ui` pass.
- **test**: `npm run check && npm run test:ui`
- **routine: yes** (two files, exact content given)

### Task 9: Tree building, sorting and fuzzy filtering (pure TS)
- **files**: `src/ui/lib/fuzzy.ts`, `src/ui/lib/fuzzy.test.ts`, `src/ui/lib/library-tree.ts`,
  `src/ui/lib/library-tree.test.ts` (all new), `src/ui/lib/fixture-tracks.ts` (new: the six
  fixture tracks of Task 4 as `TrackRecord[]`, shared by the frontend tests)
- **does**: As §2.8 "Tree" and "Search". `buildTree(tracks) -> BandNode[]` (sorted, grouped),
  `allGroupKeys(tree)`, `filterTree(tree, query) -> { tree, expandKeys }`,
  `normalize`, `tokenMatches(token, field)`, `trackMatches(track, query)`. Tests on the
  fixture: band order `Amber Fields, the Night Owls, Zephyr Lane, (no band)`; Amber albums
  `Copper Sky, Northern Roads`; Northern Roads tracks `after midnight, Slow Burn`;
  "amber fields" and "Amber Fields" group together; every query example in §2.8 gives the
  stated result; the empty query returns the full tree; whitespace-only = empty.
- **done when**: `npm run check && npm run test:ui` pass.
- **test**: `npm run check && npm run test:ui`
- **routine: no** (algorithm)

### Task 10: Track draft model and button states (pure TS)
- **files**: `src/ui/lib/track-draft.ts`, `src/ui/lib/track-draft.test.ts` (new)
- **does**: `Draft` from a `TrackRecord` (form strings: composers joined by `\n`, year as a
  string); `isDirty`; `validate(draft) -> Record<field, message>` (mirrors the Rust rules:
  title required, year 1..9999 integer, lengths); `toEdits`; tab staging:
  `tabRows(draft)` → `{key, name, state: 'saved'|'new'|'updated'|'missing'}`,
  `stageAdd(draft, picked, audio) -> Result` (clash check case-insensitive, `track.json`,
  audio name), `stageUpdate(draft, key, picked)`, `stageRemove(draft, key)`;
  `toSaveRequest(draft)`; `paneButtons(mode, hasSelection, busy)` and
  `tabButtons(mode, selectedRow | null)` returning `{edit, save, cancel, export, delete}` /
  `{add, update, export, remove}` booleans, exactly as §2.8 (AC8, AC9, AC12, AC13). Tests
  cover every row of those rules, a staged add → remove → add again, an update of a "new" row
  (becomes a new add with the new token), and request building (`keep` / `add` /
  `replace`, removed names omitted, order kept).
- **done when**: `npm run check && npm run test:ui` pass.
- **test**: `npm run check && npm run test:ui`
- **routine: no**

### Task 11: Add shadcn `textarea` and `alert-dialog`; ConfirmDialog component
- **files**: `src/ui/lib/components/ui/textarea/**`, `src/ui/lib/components/ui/alert-dialog/**`
  (generated), `src/ui/components/ConfirmDialog.svelte`, `src/ui/components/ConfirmDialog.test.ts`,
  `package.json`/`package-lock.json` only if the CLI changes them
- **does**: `npx shadcn-svelte@1.7.0 add textarea alert-dialog -y` (fix paths by hand if
  needed, like `components.json` before). Check that their data attributes are covered by the
  `@custom-variant` mappings in `app.css` (add a mapping, plus an assertion in
  `tabs-variants.test.ts`, if alert-dialog uses one that isn't there). `ConfirmDialog` props:
  `open` (bindable), `message`, `onanswer(yes: boolean)`; buttons "No" (initial focus) and
  "Yes"; Escape = No. Test: renders the message, Yes/No call back with true/false, initial
  focus is on No.
- **done when**: `npm test` passes (includes the no-static-style and CSP static tests).
- **test**: `npm test`
- **routine: no** (CLI quirks, variant check)

### Task 12: Library view: left column (toolbar, tree, search, problems)
- **files**: `src/ui/lib/library-state.svelte.ts`, `src/ui/components/library/TreeToolbar.svelte`,
  `src/ui/components/library/TrackTree.svelte`, `src/ui/views/LibraryView.svelte`,
  `src/ui/views/LibraryView.test.ts` (new), `src/ui/App.test.ts`,
  `src/ui/acceptance_frontend.test.ts`
- **does**: The `library-state` store and `load()` (calls `listTracks`, logs the `library …`
  line, handles errors and the `missing`/`newer` statuses with a message and a pointer to
  Settings). LibraryView: `<h1 id="view-heading">Library</h1>`, then two columns; the right
  column renders a placeholder `<section aria-label="Track">` for now. Toolbar buttons with
  icons, visible labels "Collapse all", "Expand all", an `Input` with `aria-label="Search tracks"`,
  and a clear button `aria-label="Clear search"` (disabled when the box is empty). The tree
  and keyboard behaviour follow §2.8; `Ctrl+F`, and ↓ from the search box. Empty library:
  "No tracks in the repository yet." plus the root path. Problems line. Update the old shell
  tests: their `mockIPC` handlers return an empty library for `list_tracks` and
  `{root:'/tmp/x', is_default:true, status:'ok'}` for `get_repository`, and
  `acceptance_frontend.test.ts` no longer expects the Library placeholder text (drop `Library`
  from `FEATURES`, with a comment that gui-tracks-repository replaced the placeholder).
  Component tests (mockIPC returning `fixture-tracks`): AC1 (every band is collapsed at first:
  only 4 treeitems visible, all `aria-expanded="false"`), AC2 (labels and order at all three
  levels after Expand all), AC3, AC4, AC5 (typing `nght` one key at a time filters on each
  input event; Clear restores the collapsed state), keyboard navigation, the empty state,
  the problems line.
- **done when**: `npm test` passes.
- **test**: `npm test`
- **routine: no**

### Task 13: Track pane and Tablature Files panel
- **files**: `src/ui/components/library/TrackPane.svelte`,
  `src/ui/components/library/TablaturePanel.svelte`, `src/ui/views/LibraryView.svelte`,
  `src/ui/lib/library-state.svelte.ts`, `src/ui/views/TrackPane.test.ts` (new)
- **does**: Everything in §2.8 "Pane" and "Tablature panel", using `track-draft.ts` for state
  and rules and `ConfirmDialog` for the confirmations. Keep the confirmation texts as exported
  constants in `track-draft.ts`
  (`confirmRemoveTab(name)`, `confirmDeleteTrack(title)`, `CONFIRM_DISCARD`), so the wording
  lives in one place. Ctrl+E / Ctrl+S / Escape. Inline
  status/alert region. Component tests with `mockIPC` (handlers return fixture records and
  record the `save_track` requests): AC6, AC7 (fields visible and disabled, buttons visible and
  disabled), AC8 (Edit/Export/Delete enabled, Save disabled, the bar is after the fields in
  DOM order), AC9, AC10 (the save request carries the edits; the shown Modified equals the
  returned record's value; back to view mode), AC11 (track 1 lists 2 tabs), AC12 (track 2: only
  Add enabled), AC13, AC14 (`pick_tablature` called with `add`), AC15 (picked
  `{token:'p1', name:'riff.gp5'}` → row "riff.gp5" selected, Remove enabled), AC16 (request
  contains `{kind:'add', token:'p1'}`), AC17 (dialog text), AC18 (Yes removes the row; no IPC
  call), AC19 (request omits the removed name); a name clash shows an error; a cancelled pick
  changes nothing; a save error stays in edit mode with the message; Delete confirms, then calls
  `delete_track`; Export shows "Exported to …"; Track 5 (5 tabs) renders a scrollable list
  (class check: max height fits 4 rows).
- **done when**: `npm test` passes.
- **test**: `npm test`
- **routine: no**

### Task 14: Settings: Track repository card
- **files**: `src/ui/components/RepositorySettings.svelte` (new),
  `src/ui/components/RepositorySettings.test.ts` (new), `src/ui/views/SettingsView.svelte`,
  `src/ui/lib/library-state.svelte.ts` (`reset()` export)
- **does**: As §2.8 "Settings card": it shows the path from `get_repository`, plus "(default)" and
  the status. Choose → confirm for `other` → `set_repository_root`. Use default →
  `reset_repository_root`. Both clear the library state and log
  `repository root=<path> default=<bool>`. Errors are shown inline. Tests with mockIPC: shows
  the path and "(default)"; cancel does nothing; `other` asks first (No → no
  `set_repository_root` call); `newer` shows an error; Use default calls the reset.
  The existing Settings tests still pass (Appearance first, then Track repository).
- **done when**: `npm test` passes.
- **test**: `npm test`
- **routine: no**

### Task 15: GUI e2e tests for the Library on display :1
- **files**: `tests/gui_library_e2e.rs` (new), `package.json` (`test:gui` script)
- **does**: Change `test:gui` to
  `npm run build && cargo test --features e2e-hooks --test gui_smoke --test gui_e2e --test gui_library_e2e -- --ignored --test-threads=1`.
  New `#[ignore]` tests (reuse the harness from `gui_e2e.rs`: copy the helpers, or move them
  into `tests/common/mod.rs` and use them from both files). Each test copies
  `tests/fixtures/library-sample` into `target/gui-e2e/<test>/data/calliope` (the default
  root under the temp `XDG_DATA_HOME`) and asserts at the start that the root is under
  `target/`. They float the window at 1280x800, fail on any `csp-violation` or `not allowed`
  stderr line, and save screenshots to `target/gui-shots/library-*.png`:
  1. `library_starts_collapsed`: the `library … tracks=6 problems=0` line; screenshots of the
     collapsed tree, then after Expand all (Ctrl+F, Shift+Tab to "Expand all", Enter).
  2. `search_filters`: Ctrl+F, type `nght`, screenshot; Escape/clear.
  3. `edit_and_save`: select "Slow Burn" (Ctrl+F, type `slow burn`, ↓, Enter); Ctrl+E,
     screenshot; change the Album field (Tab to it, select all, type); Ctrl+S; wait for
     `saved id=…01`; assert `track.json` on disk has the new album, a `modified` newer than
     the fixture value, and the unchanged `imported`; screenshot in view mode.
  4. `tablature_add_and_remove` (scripted dialog): write a temp `riff.gp5` outside the repo;
     answers file `add-tablature <path>`; on track 2: Ctrl+E, Tab to Add, Enter; wait for the
     `dialog kind=add-tablature result=picked` line; screenshot; Ctrl+S; assert the file was
     copied into the track folder, `track.json` lists it, and the source still exists. Then
     Ctrl+E, select the row, Remove, confirm Yes, Ctrl+S; assert it's gone from `track.json`,
     from the track folder, and present under `trash/*-tablatures/`.
  5. `add_opens_real_file_dialog` (no answers env var): on track 2, Ctrl+E, Add → wait for an X
     window named `Add tablature` (`xdotool search --sync --name`), `gui-shot library-dialog.png "Add tablature"`,
     press Escape in it, wait for `result=cancelled`, and check the list is unchanged.
  6. `delete_track_to_trash`: select track 4, Tab to Delete, Enter, then Tab + Enter on Yes; the
     folder is gone from `tracks/` and present under `trash/`.
  7. `settings_change_root` (scripted `repository-root <temp dir with a second copy holding only
     track 6>`): Alt+6, choose, confirm; `settings.json` has `repository_root`; Alt+1 →
     `tracks=…` count of the new root.
  8. `library_min_size`: resize to 1024x640 (i3 floating); screenshots in view mode and edit
     mode of track 5.
  The existing `gui_e2e` tests must still pass with the default root under the temp
  `XDG_DATA_HOME` (the Library now loads at start).
- **done when**: `DISPLAY=:1 npm run test:gui` passes (all old and new tests), and the
  screenshots exist.
- **test**: `npm test && DISPLAY=:1 npm run test:gui`
- **routine: no**

### Task 16: Visual review on :1
- **files**: fixes as needed in `src/ui/**`; `docs/ui.md` "Decided by the team" if any visual
  choice changes
- **does**: Look at every `target/gui-shots/library-*.png` and `settings-*.png` (take more
  with `gui-shot` as needed, dark **and** light theme). Checklist: the two columns are visible;
  the tree is readable at the 18 px root size, with clear expand chevrons; the selected track
  is highlighted in amber; the read-only vs editable states can be told apart; disabled buttons
  look disabled; the action bar sits at the bottom of the pane and is not cut off at 1024x640;
  the tablature list shows 4 rows and a scrollbar for track 5; the "new"/"updated" markers can
  be read; the confirm dialog is centred and readable; the real GTK dialog shows the
  "Tablature files" filter; nothing overlaps at 1024x640 or when maximised at 1920x1200; no
  CSP violations.
- **done when**: every checklist item is PASS, with screenshots named in the log; `npm test`
  and `DISPLAY=:1 npm run test:gui` still pass after the fixes.
- **test**: `npm test && DISPLAY=:1 npm run test:gui`
- **routine: no** (visual)

### Task 17: Licence record
- **files**: `docs/licences.md` (new), `tests/frontend.rs`
- **does**: A table of every direct dependency in `Cargo.toml` (`[dependencies]`,
  `[build-dependencies]`, `[dev-dependencies]`) and `package.json` (`dependencies`,
  `devDependencies`), with its licence (from `cargo metadata` / `node_modules/<pkg>/package.json`),
  plus a line for the bundled Inter font (OFL-1.1) and a note on the shadcn-svelte copied
  components (MIT). Static test: every dependency name appears in `docs/licences.md`.
- **done when**: `cargo test --test frontend` passes.
- **test**: `cargo test --test frontend`
- **routine: no** (needs lookups)

### Task 18: README section
- **files**: `README.md`
- **does**: Insert the following section directly before the `## Settings files` heading,
  and add the line `* \`~/.local/share/calliope/\`: default track repository (see above)` as the
  last bullet of the `## Settings files` list. Change nothing else.

  ```markdown
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
  ```
- **done when**: `grep -c "## Track repository" README.md` prints `1` and `cargo test` passes.
- **test**: `grep -c "## Track repository" README.md && cargo test --test frontend`
- **routine: yes** (exact text given)

Order: 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10 → 11 → 12 → 13 → 14 → 15 → 16 → 17 → 18.
Tasks 8-11 depend only on the types and can run before 7. Tasks 17 and 18 can run any time
after 7.

## 4. Test strategy

- **Rust unit tests** (`cargo test`): `fsutil`, `track_meta`, `settings`, `picker` (registry,
  scripted picker). Pure functions, temp dirs only.
- **Rust repository tests on temp repositories** (`src/repository_tests.rs`): every save,
  delete and export path, conflicts, rejection of path traversal, failure cleanup, and the
  "outside folder unchanged" sentinel. The committed fixture `tests/fixtures/library-sample/`
  is always copied to a temp dir before use, and never modified in place.
- **Static tests** (`tests/frontend.rs`): command list = build.rs list = capability; no
  core/plugin permissions; `e2e-hooks` absent from release scripts and default features;
  licences complete; the existing CSP/static-style tests.
- **Frontend unit/component tests** (vitest + jsdom + @testing-library/svelte, IPC faked with
  `mockIPC`): fuzzy matching, tree build/sort/filter, draft model and button states,
  ConfirmDialog, LibraryView, TrackPane/TablaturePanel, RepositorySettings. One test per
  acceptance criterion (see §7). Type check with `svelte-check`.
- **GUI e2e on display `:1`** (`tests/gui_library_e2e.rs`, `#[ignore]`d, binary built with
  `--features e2e-hooks`): the real app with temp `XDG_CONFIG_HOME`/`XDG_DATA_HOME`/`XDG_CACHE_HOME`,
  the repository a copy of the fixture under `target/gui-e2e/`, driven by `xdotool` keys,
  asserting on the `calliope-ui:` / `calliope: dialog` stderr lines **and on the files on
  disk**, with screenshots in `target/gui-shots/`. Native dialogs: the scripted picker
  answers them (no window appears). One test opens the **real** GTK dialog, takes a
  screenshot of it and cancels it with Escape, so nothing is picked. The dialog starts in the
  user's home folder, but nothing in it is read or changed; the screenshot stays local in
  `target/gui-shots/`.
- **Never touched**: the user's real repository, `~/.config/app.calliope.gui`, the OS trash,
  other displays, other hosts. There is no hardware or network in this feature.
- **Whole suite**: `npm test` (headless), then `DISPLAY=:1 npm run test:gui`.

## 5. Deployment

Single machine (the laptop): the local build only (`npm run build:app`). On the first start,
the default repository `~/.local/share/calliope/` is created (marker + `tracks/`) when the
Library is first shown.

## 6. Manual checks (owner, real desktop)

1. `npm ci && npm run build:app`, then start `target/release/calliope-gui`. The Library shows
   "No tracks in the repository yet." and `~/.local/share/calliope/` now contains
   `calliope-repository.json` and an empty `tracks/`.
2. `cp -r tests/fixtures/library-sample ~/calliope-demo`. Settings → Track repository →
   Choose folder… → `~/calliope-demo`. Library: 4 collapsed bands, sorted (Amber Fields, the
   Night Owls, Zephyr Lane, (no band)).
3. Expand all / Collapse all. Type `nght`, then `brn`, then `cafe`: the list narrows with each
   key. Clear restores it.
4. Select "Slow Burn": the fields are read-only, and Edit/Export/Delete are active while Save is
   not. Edit → change Album → Save. The Modified time is now. Open
   `~/calliope-demo/tracks/…0001/track.json` in an editor and see the change.
5. Edit → Add → the file dialog shows only tablature files. Pick a real `.gp5` from your
   collection → it appears in the list, selected → Save. The file is copied into the track
   folder, and your original stays where it was.
6. Edit → select it → Remove → the confirmation names the file → Yes → Save. The file is now
   in `~/calliope-demo/trash/`, not deleted.
7. Export the track to `/tmp` and look at the folder. Export a tablature and open it in
   TuxGuitar.
8. Delete "Lanterns" → Yes. It is moved to `trash/`.
9. Edit the album of a `track.json` by hand while that track is in edit mode in Calliope, then
   press Save: Calliope refuses with a "changed on disk" message, and your hand edit is kept.
10. Shrink the window to 1024x640: the action bar and the tablature panel are still usable.
    Read it from 1-2 m in a dim room.
11. Settings → Use default. The Library shows the default repository again. Remove
    `~/calliope-demo` when done.

## 7. Acceptance mapping

| Acceptance criterion | Task(s) | Test(s) |
|---|---|---|
| Library + tracks → list populated, collapsed | 3, 12, 15 | `LibraryView.test.ts` "AC1 starts collapsed"; e2e `library_starts_collapsed` (+ screenshot) |
| Three-level tree, each level sorted case-insensitively | 9, 12 | `library-tree.test.ts` order tests; `LibraryView.test.ts` "AC2"; e2e screenshot after Expand all |
| Expand all expands everything | 12 | `LibraryView.test.ts` "AC3"; e2e `library_starts_collapsed` |
| Collapse all collapses everything | 12 | `LibraryView.test.ts` "AC4" |
| Search filters per keystroke, case-insensitive, fuzzy (req 12) | 9, 12 | `fuzzy.test.ts`, `library-tree.test.ts`; `LibraryView.test.ts` "AC5"; e2e `search_filters` |
| Empty list → fields visible, inactive | 12, 13 | `TrackPane.test.ts` "AC6" |
| No selection → fields visible, inactive | 13 | `TrackPane.test.ts` "AC7" |
| Selection → metadata + Edit/Save/Export/Delete at the bottom, all but Save active | 10, 13 | `track-draft.test.ts` paneButtons; `TrackPane.test.ts` "AC8"; Task 16 screenshot |
| Edit → fields editable except id; Edit/Export/Delete off; Save on; Tablature panel on | 10, 13 | `TrackPane.test.ts` "AC9"; e2e `edit_and_save` screenshot |
| Save → written to disk, modified = save time, view mode, buttons restored | 2, 3, 7, 13 | `repository_tests` save tests; `TrackPane.test.ts` "AC10"; e2e `edit_and_save` (checks `track.json`) |
| Edit mode + tabs in metadata → listed | 13 | `TrackPane.test.ts` "AC11" |
| Edit mode, no tabs → only Add active | 10, 13 | `track-draft.test.ts` tabButtons; `TrackPane.test.ts` "AC12" |
| Tab selected → Update/Export/Delete(Remove) active | 10, 13 | `track-draft.test.ts`; `TrackPane.test.ts` "AC13" |
| Add → file dialog filtered to tablatures | 6, 7, 13 | `picker` filter unit test; `TrackPane.test.ts` "AC14"; e2e `add_opens_real_file_dialog` (+ screenshot) |
| Picked → name without path added, selected; Delete(Remove) active | 10, 13 | `TrackPane.test.ts` "AC15"; e2e `tablature_add_and_remove` |
| Save after add → copied, metadata lists it, timestamp updated | 3, 13 | `repository_tests` add test; `TrackPane.test.ts` "AC16"; e2e `tablature_add_and_remove` (disk) |
| Delete(Remove) → confirm "Delete … <name>" Yes/No | 11, 13 | `ConfirmDialog.test.ts`; `TrackPane.test.ts` "AC17" (`Delete tablature "<name>"?`, owner decision) |
| Yes → removed from the list, not yet from the metadata | 10, 13 | `TrackPane.test.ts` "AC18" (no `save_track` call, file still on disk in e2e before Save) |
| Save after remove → file removed from the repository (to trash), metadata updated, timestamp | 3, 13 | `repository_tests` remove test; `TrackPane.test.ts` "AC19"; e2e `tablature_add_and_remove` |
| Req 1 metadata fields, req 2 root configurable + default, req 3-6 layout | 2, 3, 5, 7, 14 | `track_meta` tests; `settings` tests; `RepositorySettings.test.ts`; e2e `settings_change_root` |

## 8. Assumptions (defaults chosen; tell us to change any)

- No conversion to mp3 happens in this feature. The audio file is shown read-only and
  is not validated by format. The mp3 default applies to the import/stem features, which create
  the audio files.
- Export track = a new folder `"Band - Album - Title"` in a chosen destination with copies of
  `track.json`, the audio and the tablatures (not a zip).
- Tablature "Update" = replace the selected tablature file with a newly picked file. The old
  file goes to the trash on Save.
- The tablature button is labelled **Remove** (req 13). The acceptance criteria's "Delete"
  button in the Tablature panel is this button.
- Tablature file types: `.gp .gp3 .gp4 .gp5 .gpx .tg .ptb .musicxml .mxl .xml`.
- Composers are edited one per line. "Source link" is plain text, not clickable.
- While editing, the tree and search are locked. The unfinished edit survives switching
  views.
- Fuzzy search = substring or word-start subsequence per token, over band/album/title (§2.8).
- Two Calliope instances on one repository are detected on save (revision conflict); there
  is no locking or live refresh. The Library rescans each time it is shown.
- Year is optional, 1-9999. Title is required; band and album may be empty.
- Track id and folder naming (open in the spec; the owner didn't object): UUIDv7 ids, folder
  name = id, flat `tracks/<id>/` with no first-letter sharding. Hand-made folders with simple
  names (`[A-Za-z0-9_-]`) are accepted too.

## 9. Open questions

None.

## 10. Owner decisions (2026-10-05)

1. **Delete on disk**: deleting a track and removing or replacing a tablature move the files to
   `<root>/trash/`. Calliope never empties it.
2. **Tablature removal text**: `Delete tablature "<name>"?` with Yes/No.
3. **Leaving edit mode**: a **Cancel** button and Escape. When there are unsaved changes, they
   first ask "Discard your changes?" (Yes/No).
4. **No track import in this feature**: tracks come from the later import/stem features or
   hand-made folders. The sample repository (`tests/fixtures/library-sample`) is for trying it out.
