# Plan: Library: album and track order, deleting tracks and albums

Spec: `specs/gui-library-album-order.md`. Base: `f45e2da` on branch `spec/gui-library-album-order`.

## 1. Summary

Each track gets two optional numbers in its `track.json`: `position` (its place in the album) and
`album_position` (its album's place among the band's albums). The Library tree sorts by these numbers,
then alphabetically, and shows "3. Hold On". In the Library tree, tracks and albums can be reordered
by pointer drag and drop or with Alt+Up / Alt+Down. Tracks and whole albums can be deleted from the
tree (a Delete button above it, plus the Delete key). Imports take the position from the file's
track-number tag. Editing a track's band or album renames the whole band or album, after a
confirmation. Rust gets two batch commands, `update_tracks` and `delete_tracks`, plus an optional
`group` in `save_track`. The frontend works out which tracks a reorder or rename touches, from the
tree the user sees.

## 2. Design

### 2.1 What is reused, what is new

| Area | Reused | New / changed |
|------|--------|---------------|
| `track.json` | schema 2, lenient read / strict write, unknown fields kept | optional `position`, `album_position` (additive, **no schema bump**, 2.2) |
| Repository | `load` + revision check, `write_atomic`, `move_track_to_trash`, the save transaction | `TrackPatch`, `prepare_patches` / `commit_patches` / `update_tracks`, `delete_tracks`; `save_track` sets the main track's positions (2.4) |
| IPC | `save_track`, `delete_track`, `list_tracks` | `update_tracks(patches)`, `delete_tracks(items)`; `save_track` request gains `position`, `album_position`, `group`; `SaveResult` gains `updated`, `failed` (2.5) |
| Import | `prepare_file`, `build_meta`, batch runner | the position from the `track` tag goes through the job into `build_meta` (2.3) |
| Tree | `buildTree`, `filterTree`, roving tabindex, `lib` state | ordering + labels (2.6), `library-order.ts` (reorder / drop / neighbour helpers), `library-rename.ts` (rename plan, 2.8), `library-ops.svelte.ts` (async reorder / delete), pointer drag and drop + keys in `TrackTree` (2.7), Delete button in `TreeToolbar` |
| Track pane | `TrackFields` read-only rows, `ConfirmDialog` | read-only "Position" row; rename confirmation before Save (2.8) |

Reordering, deleting and the Delete button only exist in the **Library**'s tree (`TrackBrowser`
`editable` prop, set only by `LibraryView`). The Editor and Player use the same tree component and show
the same order and numbers, read-only (open question 9). Their layout and tests are unchanged.

### 2.2 Data model: `track.json` (requirement 1, constraint "older files keep working")

Two optional fields are added to `TrackMeta` in `src/track_meta.rs`:

```rust
pub const MAX_POSITION: u32 = 9999;
// after `album`:
/// The album's place among the band's albums (1-based). Every track of an album carries it; the
/// tree uses the smallest value found among the album's tracks. `None` = not ordered.
#[serde(default, deserialize_with = "lenient_position", skip_serializing_if = "Option::is_none")]
pub album_position: Option<u32>,
// after `title`:
/// The track's place in its album (1-based). `None` = not ordered.
#[serde(default, deserialize_with = "lenient_position", skip_serializing_if = "Option::is_none")]
pub position: Option<u32>,
```

- **No schema bump.** Schema-2 builds from before this feature keep both keys in `extra`, write them
  back unchanged, and show the tracks alphabetically. That follows the `backings` precedent. Bumping to
  3 would make every reordered track read-only ("written by a newer Calliope") for those builds, for
  no gain.
- **Lenient read.** `lenient_position` deserialises any JSON value. An integer from 1 to 9999 becomes
  `Some`. Anything else (0, a negative number, a string, a float, `null`) becomes `None`. A bad value
  never turns the track into a problem, and the file is not rewritten because of it. The next write
  of that track simply leaves the key out.
- **Strict write.** `validate_edit_fields` rejects `Some(0)` and values above `MAX_POSITION`
  ("position must be from 1 to 9999"). It already runs for Library saves, imports (`build_meta`) and
  patches.
- Files without the keys load with `None` and are written without them, so v1 and v2 files read and
  round-trip exactly as before (the existing `round_trip_of_the_example` stays green).
- `TrackRecord` gains `position: Option<u32>` and `album_position: Option<u32>`, always serialised
  (`null` when absent).
- **The album's position** is the minimum of `album_position` over the album's tracks, where the
  album is the tree's group: trimmed name, compared case- and diacritic-insensitively, the same as
  today. Every album reorder writes the value to all tracks of every album of the band, so the values
  agree. A track imported later into an ordered album has none, and the album keeps its place through
  the minimum. Known limit: if every track carrying the value is deleted while unordered tracks remain,
  the album loses its place and sorts alphabetically after the ordered albums. The next album reorder
  fixes this.
- Tracks in a band's "(no album)" group ignore `position` (the group is not an album, requirement 16).
  Imports never set a position on a track without an album (2.3).

### 2.3 Position from the import tag (requirement 9)

- `src/media.rs`: `pub fn position_from_tags(p: &Probe) -> Option<u32>` reads the lower-cased tag
  `track`, else `tracknumber`. Rule: the text, trimmed, must match `^0*([0-9]{1,4})(/.*)?$` and give a
  number from 1 to 9999. So "3" gives 3, "03" gives 3, "3/12" gives 3, and "0", "A3", "3 of 12" and ""
  give `None`. ffprobe reports ID3 `TRCK` and Vorbis/FLAC `TRACKNUMBER` as `track`. The unit tests check
  this on files made with `ffmpeg -metadata track=…` (2.10).
- `src/import_job.rs`:
  - `prepare_file` returns a small struct `Prepared { audio, edits, duration_s, position }` instead of
    the 3-tuple. `run_url` computes the position from the same probe. `Job` gains `position:
    Option<u32>`, set by `set_ready` (new parameter).
  - `build_meta` gains `position: Option<u32>`. After `apply_edits` it sets
    `meta.position = position.filter(|_| !meta.album.is_empty())`. If the user clears the album in the
    import edit pane, the position is dropped too.
  - `start_extraction` passes `j.position`, and `extract_cleaned` / `run_extract` pass it on.
    `album_position` is never set by an import.
- `src/import_batch.rs`: uses `Prepared` and passes `position` to `extract_cleaned`.
- An import never writes any other track, so the positions of tracks already in the album never
  change. Duplicate numbers can appear (an album ordered 1..5 plus an imported "3"). The tree breaks
  such ties by title, and the next reorder renumbers 1..n.
- The import edit pane does not show the position (requirement 10 is about the Library's track pane).
  `JobSnapshot` and `ImportEvent` are unchanged.

### 2.4 Repository operations (`src/repository.rs`)

```rust
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TrackPatch {
    pub id: String,
    pub revision: String,
    #[serde(default)] pub band: Option<String>,          // None = unchanged
    #[serde(default)] pub album: Option<String>,
    #[serde(default)] pub position: Option<u32>,
    #[serde(default)] pub album_position: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TrackRef { pub id: String, pub revision: String }
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BatchFailure { pub id: String, pub title: String, pub message: String }
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct UpdateResult { pub tracks: Vec<TrackRecord>, pub failed: Vec<BatchFailure> }
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct DeleteResult { pub deleted: Vec<String>, pub failed: Vec<BatchFailure> }
/// Opaque result of phase 1 (private fields: per track the folder, the patched meta, changed or not).
pub struct PreparedPatches { /* ... */ }

impl Repository {
    pub fn prepare_patches(&self, patches: &[TrackPatch]) -> Result<PreparedPatches, String>;
    pub fn commit_patches(&self, prepared: PreparedPatches) -> UpdateResult;
    pub fn update_tracks(&self, patches: &[TrackPatch]) -> Result<UpdateResult, String>; // both phases
    pub fn delete_tracks(&self, items: &[TrackRef]) -> DeleteResult;
}
```

- **`prepare_patches`** (phase 1, nothing written). It refuses the whole batch on the first problem:
  more than 5000 patches ("too many tracks in one change"), a duplicate id, an unknown or unreadable
  track, or a revision mismatch (`conflict: "<title>" changed on disk since it was loaded`). For each
  track it applies the patch to a clone through `track_meta::apply_patch` (new): `band` / `album` are
  trimmed, must not be empty ("a band name cannot be cleared" / "an album name cannot be cleared") and
  pass the strict text rules. Positions must be from 1 to `MAX_POSITION`. Then it runs
  `validate_for_read`. Finally it records whether the meta changed.
- **`commit_patches`** (phase 2) writes only the changed tracks, with `modified` = now and
  `write_atomic`. A write error becomes a `BatchFailure` and the others go on. It returns the record of
  every patched track (re-read after a write), so the frontend can replace them. Log lines:
  `calliope: tracks updated written=<n> unchanged=<u> failed=<k>`, plus one
  `calliope: track update failed id=<id> message=<m>` per failure.
- **`delete_tracks`** works through every item, whatever happens to the others (requirement 14): load
  (an error becomes a failure), revision check (a mismatch becomes a failure with the `conflict:` text),
  `move_track_to_trash` (an error becomes a failure with "cannot move the track to the trash: <io
  error>"). `title` is taken from the loaded meta, or empty. Log lines:
  `calliope: track deleted id=<id> trash=<folder name>`, `calliope: track delete failed id=<id>
  message=<m>`, and `calliope: tracks deleted deleted=<n> failed=<k>`. The existing `delete_track` also
  logs `calliope: track deleted id=<id> trash=<folder name>`.
- **`save_track`**: `SaveTrackRequest` gains `#[serde(default)] position: Option<u32>`,
  `#[serde(default)] album_position: Option<u32>` (`Some` = set, applied right after `apply_edits`) and
  `#[serde(default)] group: Vec<TrackPatch>` (ignored by `Repository::save_track`, and used by
  `do_save_track`, 2.5). `SaveResult` gains `updated: Vec<TrackRecord>` and `failed: Vec<BatchFailure>`
  (empty from `save_track` and `save_backing`).
- Atomicity: one patch is one `track.json` write (atomic). A batch is "checked first, then written".
  It is not atomic as a whole: an I/O error halfway leaves a partly applied order or rename. That is
  reported, and the tree shows the true state after the frontend's reload. Every repository command
  runs under the existing `RepoState` mutex, so imports and saves never interleave with a batch.

### 2.5 IPC (`src/ipc.rs`, registration in `src/gui.rs`, `build.rs`, `capabilities/main.json`)

- `update_tracks(patches: Vec<TrackPatch>) -> Result<UpdateResult, String>`: `do_update_tracks` holds
  the lock and calls `update_tracks`.
- `delete_tracks(items: Vec<TrackRef>) -> Result<DeleteResult, String>`: `do_delete_tracks`. It returns
  `Err` only for an empty list or more than 5000 items.
- `save_track(request)`: `do_save_track` holds the lock and refuses a `group` that contains the main
  id. Then it runs `prepare_patches(&request.group)?` (a conflict refuses the whole save, and nothing
  is written), then `Repository::save_track`, then `commit_patches`. It puts the results into
  `SaveResult.updated` / `failed`. If the main save fails, nothing of the group is written.
- TS (`src/ui/lib/ipc.ts`): `TrackRecord` gains `position?: number | null` and
  `album_position?: number | null`. Rust always sends both; they are optional only so that the many
  test literals stay valid. Also new: `TrackPatch`, `TrackRef`, `BatchFailure`, `UpdateResult`,
  `DeleteResult`; `SaveTrackRequest` gains `position?`, `album_position?`, `group?: TrackPatch[]`;
  `SaveResult` gains `updated?`, `failed?`. Wrappers:
  `updateTracks(patches) => invoke('update_tracks', { patches })` and
  `deleteTracks(items) => invoke('delete_tracks', { items })`.

### 2.6 Tree order and labels (requirements 2, 3, 7, 16) — `src/ui/lib/library-tree.ts`

- `AlbumNode` gains `position: number | null`: the minimum `album_position` of its tracks, or `null`
  (always `null` for the "(no album)" group).
- Tracks of a real album: first those with a position, ordered by position, then title
  (`compareText`), then id; after them the others, by title then id, as today. Tracks of a
  "(no album)" group: by title then id only.
- Albums of a band: real albums with a position first (by position, then label), then real albums
  without one (by label), then "(no album)" last. Bands stay alphabetical, with "(no band)" last.
- `export function trackLabel(t: TrackRecord, album: AlbumNode): string` returns
  `` `${t.position}. ${t.title}` `` when `!album.isNone && t.position` is set, and else `t.title`.
  Albums show no number. `TrackTree` uses it for the label. Search matching still uses the bare title
  (`trackMatches` is unchanged).
- `export const groupName` (already in the file, now exported) is used by the rename planner.

### 2.7 Reordering in the tree (requirements 4-8, 17)

**Pure helpers, `src/ui/lib/library-order.ts` (new):**

```ts
export interface Patch { id: string; revision: string; band?: string; album?: string; position?: number; album_position?: number }
/** `list` with the item at `from` moved so that it ends at index `to` (both 0-based). */
export function moveItem<T>(list: T[], from: number, to: number): T[];
/** Gap index 0..n (0 = before the first) to the final index of an item now at `from`; null = no move. */
export function gapToIndex(from: number, gap: number): number | null;   // gap > from ? gap - 1 : gap; equal to from -> null
/** Gap under the pointer: `rects` are the siblings' rows (or album blocks), top to bottom; null when y is outside [first.top, last.bottom). */
export function gapAt(rects: { top: number; bottom: number }[], y: number): number | null; // upper half of row i -> i, lower half -> i + 1
/** Patches giving every track of the album, in the new order, positions 1..n; only tracks whose position differs. */
export function trackOrderPatches(album: AlbumNode, from: number, to: number): Patch[];
/** The same for the band's real albums (the "(no album)" group excluded): album_position 1..m on every track of each album. */
export function albumOrderPatches(band: BandNode, from: number, to: number): Patch[];
/** The first key of `before` after the removed ones that still exists in `after`, else the nearest one before them, else after[0] ?? null. */
export function neighbourKey(before: string[], removed: Set<string>, after: string[]): string | null;
export const confirmDeleteAlbum = (band: string, album: string, n: number): string =>
  `Delete the album "${album}" by ${band} (${n === 1 ? '1 track' : `${n} tracks`})? The tracks will be moved to the repository's trash folder.`;
```

`band` in `confirmDeleteAlbum` is the band's tree label ("(no band)" for the no-band group).

**Async operations, `src/ui/lib/library-ops.svelte.ts` (new; imports `lib`, `load`, `fullTree`):**

- `reorderAllowed(): string | null` returns the reason a reorder is refused, else `null`: `'search'`
  (search text present), `'edit'` (edit mode), `'busy'`.
- `moveTreeItem(key: string, delta: -1 | 1, via: 'key'): Promise<void>` and
  `dropTreeItem(key: string, gap: number, via: 'drag'): Promise<void>` both find the item in
  `fullTree()`:
  - A track key `t:<id>` in a real album moves among that album's tracks. An album key that is not
    "(no album)" moves among the band's real albums.
  - Anything else does nothing: a band, the "(no album)" group or its tracks, the first item moving up,
    the last moving down, a gap that changes nothing, or a refusal from `reorderAllowed()`. It logs
    `reorder none key=<key> via=<v> reason=<search|edit|busy|band|no-album|first|last|same|outside>`.
  - Otherwise it builds the patches (`trackOrderPatches` / `albumOrderPatches`) and sets `lib.busy`.
    It calls `updateTracks` (skipped when the list is empty), replaces the returned records in
    `lib.library.tracks`, and logs
    `reorder tracks album=<album key> id=<id> from=<i> to=<j> via=<v> written=<n> failed=<k>` or
    `reorder albums band=<band key> album=<album key> from=<i> to=<j> via=<v> written=<n> failed=<k>`
    (i and j 1-based). On failures it sets
    `lib.errorMessage = "Could not save the new order for N track(s): <title>: <message>; …"` and calls
    `load()`. A `conflict:` error makes it call `load()` and set
    `lib.errorMessage = 'The album changed on disk and was reloaded. Try again.'`
- Key presses that arrive while `lib.busy` is set are dropped (logged with `reason=busy`). A held
  Alt+Down therefore moves one place per finished save. Saving is local and quick.

**`TrackTree.svelte`** (prop `editable = false`; `TrackBrowser` passes it through, and only
`LibraryView` sets it):

- Keys (only when `editable`): Alt+ArrowUp / Alt+ArrowDown on a focused item call `moveTreeItem` and
  then re-focus the same key (Svelte's keyed `{#each}` moves the DOM node, which can drop the focus).
  Plain `Delete` calls the `ondelete` callback (2.9). Both are handled before the existing
  "any modifier: return" guard and call `preventDefault`.
- The focused key moves from the component's local `focusKey` to the shared `lib.treeFocus` (the
  Delete button needs it). It is reset by `resetLibrary`.
- **Pointer drag and drop** (open question 6: pointer events, not HTML5 drag and drop):
  - `pointerdown` (primary button, `editable`, a track of a real album or a real album row, and
    `reorderAllowed()` is `null`) records the start. The drag starts only after the pointer has moved
    5 px vertically, so a plain click still selects or toggles. While a drag runs, the component listens
    on `window` for `pointermove`, `pointerup`, `pointercancel` and `keydown` (Escape). That works in
    WebKitGTK and in jsdom, with no pointer capture.
  - On each move it computes the gap with `gapAt` over the siblings' rows (`getBoundingClientRect` of
    the `[data-key]` rows). For tracks, the siblings are that album's track rows. For albums, the
    siblings are blocks, each from the album row's top to the bottom of its last visible track row (or
    the album row's own bottom when collapsed). A pointer left or right of the tree root, or above or
    below the siblings, gives `null` (no target).
  - Drop marker: the row at the gap gets `data-drop="before"` (an inset 2 px primary line at its top),
    or the last sibling gets `data-drop="after"` (the line at its bottom). This uses Tailwind
    `shadow-[inset_…]` classes (CSP-safe, no layout shift). The dragged row is shown at 50 % opacity,
    and the tree gets `select-none` and `cursor-grabbing`. On every change of target it logs
    `drag target key=<key> gap=<g|none>`. The drag start logs `drag start key=<key>`.
  - `pointerup` with a target calls `dropTreeItem(key, gap, 'drag')`. Without a target, or after
    Escape / `pointercancel`, it logs `reorder none key=<key> via=drag reason=outside` and nothing
    changes. The `click` that follows a drag is swallowed.
  - Auto-scroll: while dragging within 24 px of the top or bottom edge of the scrolling tree column
    (`TrackBrowser`'s `overflow-y-auto` element, passed in as a prop), the column scrolls 8 px per
    animation frame.
- For screen readers, the tree's `aria-describedby` points
  to a hidden hint: "Alt+Up and Alt+Down move the focused track or album; Delete deletes it". That
  hint appears only when `editable`.

### 2.8 Renaming a band or album from the track pane (requirement 11, spec Q1-Q3)

**Pure planner, `src/ui/lib/library-rename.ts` (new):**

```ts
export type RenamePlan =
  | { kind: 'none' }                                        // band and album unchanged: a plain save
  | { kind: 'refused'; reason: 'clear-band' | 'clear-album' | 'both'; message: string }
  | { kind: 'join'; position?: number; album_position?: number }                     // spec Q3
  | { kind: 'rename'; scope: 'band' | 'album'; from: string; to: string; count: number; merged: boolean;
      message: string; main: { position?: number; album_position?: number }; group: Patch[] };
export function planRename(tracks: TrackRecord[], edited: TrackRecord, band: string, album: string): RenamePlan;
```

`band` and `album` are the draft values. "Changed" means the trimmed draft differs from the trimmed
record value, compared exactly, so a change of case alone counts (spec Q2). "Shown order" means the
order of `buildTree(tracks)`.

1. Neither changed: `none`.
2. Both changed: `refused`, reason `both`, message "Change the band or the album, save, then change
   the other." (open question 5).
3. A non-empty band cleared: `refused`, `clear-band`, message "The band name cannot be cleared." A
   non-empty album cleared: `refused`, `clear-album`, message "The album name cannot be cleared."
   (spec Q3).
4. **Album changed, and the track is in "(no album)"** (spec Q3): `join`. Only this track changes. If
   the target album (same band, `groupName` match) exists and has positioned tracks, then
   `position` = max + 1. If it has an album position, `album_position` = that value. A new album gets
   neither. No confirmation.
5. **Album changed, track in a real album**: `rename`, scope `album`. Affected: every track of the
   source album group (same band key, same album key). `count` = their number.
   - If the new name's `groupName` is the source's own key (a change of case or accents only), it is
     not a merge: every affected track gets `album: <new>`, and the positions stay.
   - If another album of the same band has that key, it is a **merge** (spec Q1). Let E be the existing
     album's tracks in shown order and R the renamed tracks in shown order. If any track of E ∪ R has a
     position, E gets positions 1..|E| and R gets |E|+1..|E|+|R|. If none has, no positions are set.
     The merged album's `album_position` is E's position, else R's, else none, and it is set on every
     track of E ∪ R where it differs. Tracks of E get patches only where a position changes.
   - Otherwise it is a plain rename: `album: <new>` on every affected track, and the positions and
     `album_position` stay, so the album keeps its place (requirement 11).
6. **Band changed** (the album unchanged): `rename`, scope `band`.
   - A track in "(no band)" (open question 4): the affected tracks are the tracks of its "(no band)"
     album group, or only the track itself when it is in "(no band)" / "(no album)". They join the
     target band as if that group were renamed into it. A single affected track needs no confirmation
     (the planner returns `join` with no positions).
   - Otherwise the affected tracks are every track of the source band, including its "(no album)"
     tracks.
   - Case or accent change only: every affected track gets `band: <new>`. Nothing else changes.
   - The target band exists: **merge** (spec Q1). Albums with the same album key in both bands are
     merged with the album rule above (existing tracks first). The album list becomes the target
     band's real albums in shown order, then the source band's real albums that did not collide, in
     shown order. If any of them has an album position, all of them are renumbered 1..m in that order.
     Otherwise none is set. The "(no album)" tracks simply join the target's "(no album)" group.
   - Otherwise it is a plain rename: `band: <new>` on every affected track, with the order kept.
7. `main` holds the edited track's own position fields where they change (its band and album go with
   its edits). `group` holds the patches of all the other affected or renumbered tracks, each with its
   `revision`. A patch carries `band` / `album` only for tracks whose name changes.
8. `message`:
   - Album: `Rename the album "<from>" to "<to>" for <count> track(s)?`
   - Band: `Rename the band "<from>" to "<to>" for <count> track(s) in <a> album(s)?`
   - A merge adds: ` "<to>" already exists (<k> track(s)); they will be merged, the renamed ones after
     the existing ones.`
   - `track(s)` means "1 track" or "N tracks", and the same for albums.

**Save flow (`src/ui/lib/library-state.svelte.ts`):**

- `lib` gains `pendingRename: RenamePlan | null`, plus `treeFocus` (2.7).
- `saveEdit(confirmed = false)` validates the fields as today. Then it computes
  `planRename(lib.library.tracks, selectedTrack(), draft.band, draft.album)`:
  - `refused`: it sets `lib.errorMessage` to the message, logs `rename refused reason=<r>`, and stays
    in edit mode.
  - `rename` and not `confirmed`: it sets `lib.pendingRename = plan` and returns. `TrackPane` opens its
    `ConfirmDialog` with `plan.message`. Yes calls `saveEdit(true)`. No / Escape clears
    `pendingRename`, logs `rename cancelled`, and leaves the pane in edit mode with the draft
    (acceptance "when the user cancels, then nothing is saved").
  - Otherwise it sends `saveTrack({...toSaveRequest(d), ...plan.main, group: plan.group})`. It replaces
    `res.track` and every `res.updated` record in `lib.library.tracks`, and sets `lib.expanded[newBandKey]`
    and `lib.expanded[newAlbumKey]` to true, so the renamed track stays visible (group keys are
    name-based). For a rename it logs
    `rename kind=<band|album> from="<from>" to="<to>" tracks=<count> merged=<b> failed=<k>`. Failures
    are listed in `lib.message` after "Saved." ("Could not rename N track(s): …"), followed by
    `load()`. A `conflict:` from a group member uses the existing conflict path (edit mode kept). The
    text names the album: "Another track of this album or band changed on disk. Cancel to reload; your
    edits will be discarded."
- Ctrl+S goes through the same `saveEdit`, so it gets the same confirmation.

**Position in the track pane (requirement 10):** `TrackPane`'s read-only rows gain
`{ id: 'track-position', label: 'Position', value: position-or-'none' }` right after "Type". The value is
`none` for a "(no album)" track. It is read-only in view and edit mode alike. The editable fields
(`TrackFields`, shared with Import) are unchanged.

### 2.9 Deleting from the tree (requirements 12-16)

- **Target**: `treeDeleteTarget(): { kind: 'track'; track } | { kind: 'album'; bandLabel; album: AlbumNode; tracks } | null`
  in `library-ops`. It uses `lib.treeFocus` when that key is still in the visible tree, and else the
  selected track. A band key, a "(no album)" key, edit mode or `lib.busy` give `null`, which disables
  the button and makes the key do nothing (logged as `delete none key=<k> reason=<band|no-album|none|edit|busy>`).
  - An album's tracks come from `fullTree()`, not the filtered tree. While a search filters the
    album, the whole album is deleted, and the confirmation gives the full count (open question 7).
- **Delete button** in `TreeToolbar`: it is rendered only when an `ondelete` prop is passed (the
  Library). It sits on the Collapse all / Expand all row, as an icon button (Trash icon,
  `variant="outline"`, destructive text colour) with `aria-label="Delete"` and
  `title="Delete the focused track or album (Delete)"`, so the row does not wrap in the 20 rem column
  (open question 10). It is disabled when `treeDeleteTarget()` is `null`. It sits after Expand all in
  the Tab order. When nothing is selected or focused it is disabled and skipped, so the existing
  `library_starts_collapsed` test (Ctrl+F, Shift+Tab reaches Expand all) still works.
- **Confirmation**: `TrackBrowser` owns a `ConfirmDialog`. For a track it shows the existing
  `confirmDeleteTrack(title)`. For an album it shows `confirmDeleteAlbum(bandLabel, albumLabel, n)`.
  No / Escape logs `delete cancelled key=<k>`, and nothing is deleted.
- **Delete**:
  - A track goes through the existing `deleteTrack(id, revision)` (the same as the pane, and the same
    conflict handling), logging `deleted id=<id>`.
  - An album goes through `deleteTracks(items)`. It logs `deleted id=<id>` for each deleted track and
    `deleted album band="<band>" album="<album>" tracks=<n> failed=<k>`. Then
    `lib.message = 'Deleted the album "<album>" (<n> tracks).'`. If anything failed,
    `lib.errorMessage = 'Deleted <n-k> of <n> tracks. Could not delete: "<title>": <message>; …'` is
    used instead.
  - Both then call `load()`. If the selected track was deleted, `selectedId` becomes `null`.
- **Focus after a delete** (requirement 15): before deleting, `TrackBrowser` takes the visible keys
  from `TrackTree` (exported `visibleKeys()`). After `load()` it focuses
  `neighbourKey(before, removedKeys, after)` (exported `focusKey(key)`). The removed keys are the
  track's key, or the album's key plus its track keys. An empty album or band disappears by itself,
  because `buildTree` only makes groups that have tracks.

### 2.10 Testing aids

- **Tagged copies**: `tests/common/mod.rs` gains `tagged_copy(src: &Path, dest: &Path, tags: &[(&str, &str)])`.
  It runs the real `ffmpeg -nostdin -i file:<src> -map 0:a -c copy -metadata k=v … file:<dest>`. The
  Rust unit tests in `media.rs` / `import_job_tests.rs` use their own copy of that helper (they are
  module tests). There is no new binary fixture.
- **Seeded repositories for GUI tests**: `tests/common/mod.rs` gains
  ```rust
  pub struct Seed<'a> { pub id: &'a str, pub band: &'a str, pub album: &'a str, pub title: &'a str,
                        pub position: Option<u32>, pub album_position: Option<u32> }
  /// A temp dir (under target/) whose default root holds only these schema-2 backing tracks
  /// (each with a copy of the sample's backing.mp3) and the repository marker.
  pub fn seed_dirs(test: &str, tracks: &[Seed]) -> Dirs;
  impl App { /// Left-button drag from (x0, y0) to (x1, y1) in 10 steps of 40 ms, relative to the window.
             pub fn drag_xy(&self, wid: &str, from: (i32, i32), to: (i32, i32), before_release: impl FnOnce()); }
  ```
  Hand-made ids such as `t-a` are valid track ids. A small seeded tree has fixed row positions at
  1280x800. The GUI tests measure them once from a screenshot and keep them as constants, the same as
  the other GUI tests do.
- **A track that cannot be deleted**: in GUI tests, the test changes that track's `track.json` on disk
  after the Library has loaded (it appends a newline). The revision then no longer matches, and the
  failure reason is the `conflict:` text. Rust tests also use `chmod 0555` on the track folder (renaming
  a folder to another parent needs write access to the folder itself). That variant is skipped when
  `geteuid() == 0`, because under `unshare -r` the test would run as root.

## 3. Tasks

Rust tasks end with `cargo clippy -p calliope-gui --all-targets -- -D warnings` clean, and frontend
tasks with `npm run check` clean (both in `src/calliope-gui/`). Implementers run only the tests named
in their task. The orchestrator runs `npm test` once per task, and `npm run test:gui` where the task
says so. **Extend existing tests instead of adding near-duplicates**: the `example()` / `EXAMPLE_V2`
fixtures of `track_meta.rs`, the temp-repository helpers of `src/repository_tests.rs`, the `Rig` of
`src/import_job_tests.rs`, the `rec()` factory and `mockIPC` set-up of `src/ui/acceptance_library.test.ts`
and `src/ui/views/LibraryView.test.ts`, and the `App` helpers of `tests/common/mod.rs`. **Keep waits
short**: GUI tests wait on log lines, never on fixed sleeps beyond `settle()`. Never commit in a
routine task. Tests use temporary repositories only. They never open a real dialog or audio device,
and never use the real `calliope-stems` on `0.0.0.0:8765` or any other host.

### Task 1: `track.json` positions
- **files:** `src/calliope-gui/src/track_meta.rs`; the `TrackMeta` literals only in `src/calliope-gui/src/repository.rs`
  (`create_track`), `src/calliope-gui/src/repository_tests.rs` (`stem_meta`) and `src/calliope-gui/src/import_job.rs` (`build_meta`)
- **does:** Add `MAX_POSITION`, `album_position` (after `album`) and `position` (after `title`) to
  `TrackMeta`, with `lenient_position` (private `fn lenient_position<'de, D>(d: D) -> Result<Option<u32>, D::Error>`
  over `Option<Value>`), as in 2.2. In `validate_edit_fields`, reject positions outside 1..=9999
  (messages "position must be from 1 to 9999", "album position must be from 1 to 9999"). Add
  `pub fn apply_patch(meta: &mut TrackMeta, band: Option<&str>, album: Option<&str>, position: Option<u32>, album_position: Option<u32>) -> Result<(), String>`:
  it trims names, refuses empty names (2.4 texts), runs `validate_edit_fields` + `validate_for_read`
  on a clone, and leaves `meta` unchanged on error. Tests: absent keys stay absent after a round trip
  (`EXAMPLE`, `EXAMPLE_V2`, `STEM`); `{"position": 3, "album_position": 2}` round-trips; `0`, `-1`,
  `"3"`, `3.5`, `null` and `10000` read as `None` without making a problem; unknown fields still
  survive; `validate_for_write` rejects `Some(0)` / `Some(10000)`; `apply_patch` sets, trims, refuses
  empty names and keeps `meta` on error. The other `TrackMeta` literals get `position: None,
  album_position: None` so the crate builds.
- **done when:** `cargo test -p calliope-gui --lib track_meta` passes and clippy is clean.
- **routine:** no

### Task 2: Repository: records, patches, batch delete, positions in `save_track`
- **files:** `src/calliope-gui/src/repository.rs`, `src/calliope-gui/src/repository_tests.rs`,
  `src/calliope-gui/tests/acceptance_tracks_repository.rs` (only the `TrackRecord` literal near line 640)
- **does:** `TrackRecord.position` / `album_position` (filled in `record`). `TrackPatch`, `TrackRef`,
  `BatchFailure`, `UpdateResult`, `DeleteResult`, `PreparedPatches`, `prepare_patches`,
  `commit_patches`, `update_tracks`, `delete_tracks` and their log lines, exactly as in 2.4.
  `SaveTrackRequest` gains `position`, `album_position` and `group` (serde defaults). `save_track`
  applies the two positions after `apply_edits`. `SaveResult` gains `updated` and `failed` (empty in
  `save_track` / `save_backing`). `delete_track` logs `calliope: track deleted id=<id> trash=<name>`.
  Tests (temp repositories, the outside-folder check as in the existing tests):
  - a Library save keeps `position` / `album_position`, and an export's `track.json` contains them;
  - `update_tracks` writes only the changed tracks: an unchanged track keeps its bytes and revision, and
    `modified` changes on written ones only;
  - a stale revision or an unknown id refuses the batch, and no file changes;
  - a duplicate id, an empty band, `position: 0` and 5001 patches are refused;
  - a v1 track patched with a position is written as schema 2 with the position and all other fields
    kept;
  - `delete_tracks` of 4 tracks where one has a stale revision deletes 3, and reports the one with
    `conflict:`;
  - the same with `chmod 0555` on one track folder reports "cannot move the track to the trash"
    (skipped when `libc::geteuid() == 0`; restore the mode before the temp dir is dropped);
  - `save_track` with `position: Some(4)` sets it.
- **done when:** `cargo test -p calliope-gui --lib repository` and
  `cargo test -p calliope-gui --test acceptance_tracks_repository` pass.
- **routine:** no

### Task 3: Position from the track-number tag at import
- **files:** `src/calliope-gui/src/media.rs`, `src/calliope-gui/src/import_job.rs`,
  `src/calliope-gui/src/import_batch.rs`, `src/calliope-gui/src/import_job_tests.rs`
- **does:** `position_from_tags` (2.3) with table tests over `tags(&[("track", …)])`: "3", "03", "3/12",
  " 7 " give a number; "0", "A3", "3 of 12", "", "10000" give `None`; `tracknumber` is the fallback.
  One real-file test makes copies of `tests/fixtures/import/untagged.flac` and `tagged.mp3` with
  `-metadata track=2/9` (into a `tempfile` dir, through the local `ffmpeg`) and checks that `probe` +
  `position_from_tags` give 2 for both. `Prepared`, `Job.position`, `set_ready(…, position)` and
  `build_meta(…, position)` with the album rule, and `start_extraction` / batch passing it on (2.3).
  In `import_job_tests.rs`, extend one existing single-file test and one existing batch test (the `Rig`
  with the stub server):
  - a source tagged `track=2` with an album saves `position: 2`;
  - a source tagged with a track number but no album saves no position;
  - an untagged source saves none;
  - the batch saves the positions of its tagged files and leaves an existing track of the same album
    untouched (same bytes).
- **done when:** `cargo test -p calliope-gui --lib media` and `cargo test -p calliope-gui --lib import_job`
  pass.
- **routine:** no

### Task 4: IPC commands `update_tracks`, `delete_tracks`, `save_track` group
- **files:** `src/calliope-gui/src/ipc.rs`
- **does:** `do_update_tracks`, `do_delete_tracks` and the `#[tauri::command]` wrappers
  `update_tracks(app, patches)` / `delete_tracks(app, items)` (async + `blocking`, like
  `delete_track`). `do_save_track` runs prepare → save → commit for `request.group`, and refuses a
  group that contains the main id (2.5). Tests in `ipc.rs`'s test module, using its existing `Env` /
  `seed` helpers:
  - a save with a two-track group renames all three and returns `updated` (2);
  - a group member with a stale revision refuses the save, and none of the three files changes;
  - `do_delete_tracks` with an empty list is an `Err`.
- **done when:** `cargo test -p calliope-gui --lib ipc` passes. The command wrappers are `pub` (they are
  registered in Task 5), so clippy stays clean in between.
- **routine:** no

### Task 5: Register the two commands
- **files:** `src/calliope-gui/src/gui.rs`, `src/calliope-gui/build.rs`, `src/calliope-gui/capabilities/main.json`
- **does:** Three one-line insertions.
  - `src/gui.rs`: in `generate_handler![…]`, directly after the line `ipc::delete_track,`, add
    `ipc::update_tracks,` and `ipc::delete_tracks,` on two lines.
  - `build.rs`: in `COMMANDS`, directly after `"delete_track",`, add `"update_tracks",` and
    `"delete_tracks",`.
  - `capabilities/main.json`: in the line that contains `"allow-delete-track",`, insert
    `"allow-update-tracks", "allow-delete-tracks", ` right after `"allow-delete-track", `.
- **done when:** `cargo test -p calliope-gui --test frontend` passes (the static test that compares the
  three lists), and `cargo build -p calliope-gui` succeeds.
- **routine:** yes

- **Orchestrator note (after tasks 1-5), backend as built:** `update_tracks {patches}` returns
  `{tracks, failed}` (`tracks` = re-read records of every patched track; a failed write is only in
  `failed`); a batch problem refuses everything with an `Err` string (stale revision:
  `conflict: "<title>" changed on disk since it was loaded`; others `"<title>": <reason>`).
  `delete_tracks {items:[{id,revision}]}` returns `{deleted: string[], failed: {id,title,message}[]}`.
  `save_track` request gains `position?`, `album_position?`, `group?: TrackPatch[]`; the result gains
  `updated` and `failed` (always present). Every `TrackRecord` has `position` and `album_position`
  (`null` when absent). `apply_patch` runs the strict text rules on the whole track, so a track whose
  band/album has leading or trailing spaces refuses a reorder batch: show the error message.

### Task 6: TS IPC types and wrappers
- **files:** `src/calliope-gui/src/ui/lib/ipc.ts`, `src/calliope-gui/src/ui/lib/ipc-repository.test.ts`
- **does:** The type and wrapper changes of 2.5, placed next to `TrackRecord`, `SaveTrackRequest`,
  `SaveResult` and the `deleteTrack` wrapper. Tests (the file's existing `mockIPC` pattern):
  `updateTracks` sends `{ patches }` to `update_tracks`; `deleteTracks` sends `{ items }` to
  `delete_tracks`; `saveTrack` passes `group` through unchanged.
- **done when:** `npx vitest run src/ui/lib/ipc-repository.test.ts` and `npm run check` pass.
- **routine:** no

### Task 7: Tree order and position labels
- **files:** `src/calliope-gui/src/ui/lib/library-tree.ts`, `src/calliope-gui/src/ui/lib/library-tree.test.ts`
- **does:** Exactly 2.6: `AlbumNode.position`, the two sort rules, `trackLabel`, export `groupName`.
  Tests:
  - A, B, C without positions stay alphabetical, and `trackLabel` gives the bare titles (acceptance 1);
  - positions C=1, A=2, B=3 give C, A, B with labels "1. C", "2. A", "3. B";
  - a track without a position comes after the positioned ones (acceptance 6);
  - equal positions are ordered by title;
  - positions in "(no album)" are ignored, and the label is the bare title;
  - albums Alpha / Beta / Gamma with album positions G=1, A=2 (B none) give Gamma, Alpha, Beta, and
    "(no album)" stays last (acceptance 10);
  - the album position is the minimum over its tracks;
  - all existing `buildTree` / `filterTree` tests still pass unchanged.
- **done when:** `npx vitest run src/ui/lib/library-tree.test.ts` passes.
- **routine:** no

### Task 8: Reorder, drop and neighbour helpers
- **files:** `src/calliope-gui/src/ui/lib/library-order.ts` (new), `src/calliope-gui/src/ui/lib/library-order.test.ts` (new)
- **does:** The functions of 2.7's code block, with the semantics given there. Tests:
  - `moveItem`: up, down, to the ends, unchanged when `from === to`;
  - `gapToIndex`;
  - `gapAt`: the upper and lower halves, above the first row and below the last give `null`, an empty
    list gives `null`;
  - `trackOrderPatches`: moving C above A in A, B, C without positions gives three patches (C=1, A=2,
    B=3) with their revisions. A move in an already numbered album patches only the changed tracks;
  - `albumOrderPatches`: Gamma to the top patches every track of all three albums, and track
    positions are untouched. The "(no album)" group gets no patch;
  - `neighbourKey`: the next one, else the previous one, else the first one, else null;
  - `confirmDeleteAlbum` gives "1 track" / "4 tracks".
- **done when:** `npx vitest run src/ui/lib/library-order.test.ts` passes.
- **routine:** no

### Task 9: Rename planner
- **files:** `src/calliope-gui/src/ui/lib/library-rename.ts` (new), `src/calliope-gui/src/ui/lib/library-rename.test.ts` (new)
- **does:** `planRename` exactly as in 2.8, rules 1-8. Tests, one `describe` per rule:
  - "Masterof Puppets" with 8 positioned tracks renamed to "Master of Puppets" gives 7 group patches
    with `album` only, `count` 8, and a message naming both names and "8 tracks";
  - "Metalica" with 2 albums renamed to "Metallica" covers every track (including "(no album)" ones),
    keeps the positions, and the message says "in 2 albums";
  - a case-only change renames every track (spec Q2) and is no merge;
  - an album merge into an existing positioned album puts the renamed tracks after the existing ones
    (positions 1..n+k) and adopts the existing `album_position`;
  - a band merge with a colliding album name, and a band merge where neither band has album positions
    (no positions set);
  - a "(no album)" track joining an ordered album gets max + 1 and the album's `album_position`, and
    joining a new album gets nothing (spec Q3);
  - a "(no band)" album group moves as a whole (open question 4);
  - clearing a band, clearing an album, and changing both are refused;
  - no change gives `none`.
- **done when:** `npx vitest run src/ui/lib/library-rename.test.ts` passes.
- **routine:** no

### Task 10: Library state and operations
- **files:** `src/calliope-gui/src/ui/lib/library-state.svelte.ts`, `src/calliope-gui/src/ui/lib/library-ops.svelte.ts` (new),
  `src/calliope-gui/src/ui/lib/library-ops.test.ts` (new)
- **does:**
  - `lib.treeFocus` and `lib.pendingRename` (reset in `resetLibrary`);
  - the rename-aware `saveEdit(confirmed)` with its logs and messages (2.8);
  - `library-ops.svelte.ts`: `reorderAllowed`, `moveTreeItem`, `dropTreeItem`, `treeDeleteTarget`,
    `deleteTreeTarget(target): Promise<{ removedKeys: Set<string> }>` with the logs and messages of
    2.7 and 2.9.
  Tests use `mockIPC`, `resetLibrary` and `load()` with records from a local `rec()` factory:
  - Alt+Up-equivalent `moveTreeItem('t:a', -1, 'key')` sends the patches and updates
    `lib.library.tracks`;
  - the first item moving up, while searching, in edit mode, on a band or on "(no album)" sends nothing
    and logs `reorder none … reason=<…>` (assert the `frontend_log` calls);
  - a partial `update_tracks` failure sets `errorMessage` and reloads;
  - an album delete sends every track of the full (unfiltered) album and reports a failure with its
    title and reason;
  - `saveEdit` with an album change sets `pendingRename` and sends nothing; `saveEdit(true)` sends
    `group`; a refused plan sets `errorMessage` and stays in edit mode.
- **done when:** `npx vitest run src/ui/lib/library-ops.test.ts` and
  `npx vitest run src/ui/views/TrackPane.test.ts src/ui/views/LibraryView.test.ts` (existing, unchanged)
  pass, and `npm run check` is clean.
- **routine:** no

- **Orchestrator note (after tasks 6-10), frontend as built:** `lib.treeFocus` and
  `lib.pendingRename` are new state; `TrackTree` still has its local `focusKey` and must switch to
  `lib.treeFocus`. `saveEdit(confirmed = false)` (call as `() => void saveEdit()`; `saveEdit(true)` is
  Yes), `cancelRename()` for No/Escape. Ops in `library-ops.svelte.ts`: `reorderAllowed()`,
  `moveTreeItem(key, delta, 'key')`, `dropTreeItem(key, gap, 'drag')`, `treeDeleteTarget()`,
  `deleteTargetOrReason()`, `logDeleteNone()`, `deleteTreeTarget(target)` → `{removedKeys}` (calls
  `load()`, sets messages). Nothing logs `delete cancelled key=<k>`: the UI must. Helpers:
  `gapAt`, `neighbourKey`, `confirmDeleteAlbum` (`library-order.ts`), `trackLabel` (`library-tree.ts`).
  Four existing frontend tests that edited Band/Album to test save errors now edit Title (owner
  requirement 11 makes a Band/Album edit a group rename).

### Task 11: Tree UI: keys, pointer drag and drop, Delete button and confirmation
- **files:** `src/calliope-gui/src/ui/components/library/TrackTree.svelte`, `TreeToolbar.svelte`, `TrackBrowser.svelte`,
  `src/calliope-gui/src/ui/views/LibraryView.svelte`, `src/calliope-gui/src/ui/views/LibraryView.test.ts`
- **does:** 2.7 (component part) and 2.9 (button, dialog, focus after delete). The labels go through
  `trackLabel`. `LibraryView` renders `<TrackBrowser heading="Library" editable …>`. Component tests in
  `LibraryView.test.ts`:
  - Alt+ArrowDown on a focused track moves it, sends `update_tracks`, and the same row keeps the focus
    (`document.activeElement.dataset.key`);
  - a pointer drag (`fireEvent.pointerDown` on C, `pointerMove` / `pointerUp` on `window`, with
    `getBoundingClientRect` stubbed per row) shows `data-drop="before"` on A while moving and sends
    the C-A-B order on release;
  - a drag that ends over another album's row, or after Escape, sends nothing;
  - a plain click after `pointerdown` without movement still selects;
  - the Delete button is disabled with a band focused and with nothing focused, and enabled on a track
    and on an album;
  - the Delete key on an album opens the dialog with band, album and "4 tracks"; No sends nothing; Yes
    sends `delete_tracks` and the focus lands on the neighbour;
  - in `EditorView.test.ts` / `PlayerView.test.ts` (existing), the tree renders no Delete button, and
    Alt+ArrowUp does nothing (one assertion each, added to an existing test).
- **done when:** `npx vitest run src/ui/views` passes and `npm run check` is clean. Then
  `npm run test:gui -- library_starts_collapsed` passes (the Delete button must not break its
  Shift+Tab).
- **routine:** no

### Task 12: Track pane: position row and rename confirmation
- **files:** `src/calliope-gui/src/ui/components/library/TrackPane.svelte`, `src/calliope-gui/src/ui/views/TrackPane.test.ts`,
  `src/calliope-gui/tests/gui_library_e2e.rs` (Tab counts only)
- **does:** The "Position" read-only row after "Type" (2.8). The `pendingRename` → `ConfirmDialog`
  wiring (Yes → `saveEdit(true)`, No → clear + log). Tests in `TrackPane.test.ts`:
  - the Position row shows "3" and "none", and is read-only in edit mode (acceptance 12);
  - Save after an album change shows the confirmation text; No keeps edit mode and sends no
    `save_track` (acceptance 15); Yes sends one `save_track` with `group`.
  The new row adds one Tab stop before the tablature panel and the action bar, so in
  `tests/gui_library_e2e.rs` the following Tab counts go up by one:
  - `delete_track_to_trash` 15 → 16;
  - `tablature_add_and_remove` 14 → 15 and both 12 → 13;
  - `add_opens_real_file_dialog` 14 → 15.
  Update the comments next to them too.
- **done when:** `npx vitest run src/ui/views/TrackPane.test.ts` passes, and `npm run test:gui --
  delete_track_to_trash tablature_add_and_remove add_opens_real_file_dialog` passes. (The filter is
  passed to every binary; tests that are not matched are skipped.)
- **routine:** no

### Task 13: Headless acceptance tests
- **files:** `src/calliope-gui/src/ui/acceptance_library.test.ts`, `src/calliope-gui/tests/acceptance_tracks_repository.rs`,
  `src/calliope-gui/tests/acceptance_existing_track_import.rs`
- **does:** One test per acceptance criterion that is not already pinned by a unit test, written from
  the spec's wording:
  - Frontend (`acceptance_library.test.ts`, its `rec()` / `mockIPC` set-up): acceptance 1-10 and
    13-21 through `LibraryView`, with the fake `update_tracks` / `delete_tracks` / `save_track` applying
    the patches to its in-memory records. "After a restart" is a `resetLibrary()` plus a re-render on the
    patched records.
  - Rust `acceptance_tracks_repository.rs`:
    - "a track.json from the previous version loads unchanged, with no position": the sample (v1) and
      library-v2 fixtures are scanned, and the bytes and revision are unchanged;
    - update + delete batches against a temp copy, including the partial delete failure.
  - Rust `acceptance_existing_track_import.rs`: a directory of three `tagged_copy` files with
    `track=2`, `1`, `3`, one album and distinct titles is imported through the stub server (the
    existing `ac_*` directory rig). The saved tracks have positions 2, 1 and 3 by title, and an existing
    track of that album keeps its bytes.
- **done when:** `npx vitest run src/ui/acceptance_library.test.ts`,
  `cargo test -p calliope-gui --test acceptance_tracks_repository` and
  `cargo test -p calliope-gui --test acceptance_existing_track_import` pass.
- **routine:** no

### Task 14: GUI end-to-end tests and visual review
- **files:** `src/calliope-gui/tests/common/mod.rs` (`Seed`, `seed_dirs`, `drag_xy`, `tagged_copy`),
  `src/calliope-gui/tests/gui_library_e2e.rs`
- **does:** Four new tests on seeded repositories. Rows are reached with fixed coordinates (measured
  once with `gui-shot`) or with the keyboard. Each test waits on log lines and saves screenshots:
  - `library_reorder_tracks`: album "Order" (A, B, C) of band "Band", and band "Other" with album
    "Elsewhere" (D).
    - Expand all, then a screenshot (alphabetical, no numbers).
    - `drag_xy` C → upper half of A, with a screenshot in `before_release` (marker), after waiting for
      `drag target key=t:c gap=0`. Wait for `reorder tracks … written=3`, then the disk has C=1, A=2,
      B=3.
    - Focus A (click it). Alt+Up gives `reorder tracks id=t-a from=2 to=1`, and the disk has A=1, C=2,
      B=3. A second Alt+Up gives `reorder none … reason=first`, with the disk unchanged.
    - Drag A onto D gives `reason=outside`, with the disk unchanged.
    - Ctrl+F, type "a", Down Down Down, Alt+Down gives `reason=search`.
    - `close_gracefully` + `start_lib`, Expand all, then a screenshot ("1. A", "2. C", "3. B").
  - `library_reorder_albums`: band "Band" with albums Alpha, Beta, Gamma (2 tracks each, positioned)
    and one "(no album)" track, plus a band "Other".
    - Drag the Gamma row above Alpha. The album positions on disk become G=1, A=2, B=3, and every
      track position is unchanged.
    - Alt+Down on Alpha gives G, B, A.
    - Drag Alpha onto "Other"'s album gives `reason=outside`.
    - Screenshot: "(no album)" is last.
    - Restart, then a screenshot.
  - `library_delete_from_tree`:
    - Select a single track, press Delete, then Tab Return (Yes). The track is in `trash/` and out of
      the tree (`library root=… tracks=<n-1>`).
    - Focus an album of 4 tracks and click the Delete button. Screenshot of the confirmation, then
      Escape: `delete cancelled`, and nothing has moved.
    - Delete again; before confirming, append a newline to one of the 4 `track.json` files. Yes gives
      `deleted album … tracks=4 failed=1`; 3 folders are in `trash/` and the changed one stays.
      Screenshot of the message.
    - Focus a band (Home): the button is disabled (screenshot), and the Delete key logs
      `delete none … reason=band`.
  - `library_rename_album_for_all`: band "Metalica" with albums "Masterof Puppets" (8 positioned
    tracks) and "Ride the Lightning" (2).
    - Select a track of the first album, Ctrl+E, change the album in its field, Ctrl+S. Screenshot of
      the confirmation, then Escape: `rename cancelled`, still `mode=edit`, files unchanged.
    - Ctrl+S, Yes gives `rename kind=album … tracks=8`; all 8 are on disk with the new album and the
      same positions.
    - Then edit the band to "Metallica" and save, with Yes: all 10 tracks have the new band, with album
      and track order kept. Screenshot.
  Then review every new screenshot by eye: the marker line is visible between rows in both themes
  (switch with the existing theme key in one shot), the Delete icon button sits on the toolbar row
  without wrapping at 1280x800 and at 1024x640, and the numbers are aligned and not truncated. Fix
  layout issues found.
- **done when:** `npm run test:gui -- library_` passes. The whole `npm run test:gui` passes (the
  Editor and Player GUI tests are unchanged). The screenshots in `target/gui-shots/` are reviewed
  and described in the task's log entry.
- **routine:** no

### Task 15: README and UI guide
- **files:** `src/calliope-gui/README.md`, `docs/ui.md`
- **does:**
  - `README.md`: replace the line `Library keys: Ctrl+F search, Ctrl+E edit, Ctrl+S save, Escape
    cancel editing.` with:
    `Library keys: Ctrl+F search, Ctrl+E edit, Ctrl+S save, Escape cancel editing, Alt+Up / Alt+Down
    move the focused track or album, Delete deletes the focused track or album (after a confirmation).`
    After that line, add a paragraph:
    "Tracks are shown in album order ("3. Hold On") and albums in the order you give them. Drag a track
    between the other tracks of its album, or an album between the band's other albums, to reorder
    it (not while a search is active). Imports take the order from the file's track-number tag.
    Changing a track's band or album renames that band or album for all its tracks, after a
    confirmation."
  - `docs/ui.md`: in the "(gui-tracks-repository) Library" bullet list, after the "Library keys"
    bullet, add a bullet:
    "(gui-library-album-order) Tracks sort by album position, then title; albums by position, then
    name, with "(no album)" last. The position is shown as "3. " before the title, and albums show no
    number. Reordering uses pointer drag and drop in the Library tree only (the dragged row at 50 %,
    a 2 px primary line marks the drop gap, Escape cancels, the column scrolls near its edges), or
    Alt+Up / Alt+Down. The toolbar's Delete is an icon button after Expand all (tooltip "Delete the
    focused track or album (Delete)"). Renames are confirmed with old name, new name and track count.
    The track pane shows Position read-only after Type."
- **done when:** `grep -c "Alt+Up / Alt+Down" src/calliope-gui/README.md docs/ui.md` prints a count of
  at least 1 for each file, and `git diff --stat` shows only these two files.
- **routine:** yes

## 4. Test strategy

- **Unit (Rust)**:
  - `track_meta` (lenient read, strict write, round trips of v1 / v2 / stem files without the new keys);
  - `repository_tests` (patches, batch delete, partial failures, save and export keep positions);
  - `media` (tag parsing, plus probing real `ffmpeg` copies);
  - `import_job_tests` (single and batch imports through the stub server set positions);
  - `ipc` (save with group, conflict refuses everything).
- **Unit (frontend, vitest + jsdom)**:
  - `library-tree` (order and labels), `library-order` (patches, gaps, neighbour), `library-rename`
    (every rule) and `library-ops` (IPC calls, refusals, messages, logs);
  - component tests in `LibraryView.test.ts` / `TrackPane.test.ts`: keys, a pointer drag with stubbed
    rects, the Delete button state, the confirmations, focus after delete;
  - the Editor and Player views have no Delete button and no reordering.
- **Acceptance (headless)**: Task 13 maps every acceptance criterion to a vitest test through
  `LibraryView` (IPC faked), plus Rust tests for the old-file, batch and tagged-import criteria.
- **End-to-end (GUI, private X displays)**: Task 14 drives the real app with the real WebKitGTK.
  Pointer drag and drop is driven with `xdotool mousedown` / stepped `mousemove` / `mouseup` (the same
  way the waveform drags are tested). The other inputs are `alt+Up` / `alt+Down` / `Delete` key events,
  plus clicks at measured coordinates. The tests assert log lines and the files on disk (positions,
  `trash/`), and screenshots are reviewed for the marker, the toolbar and the labels. "After a restart"
  is a graceful close plus a new `start_lib` on the same temp dirs. The test i3 config binds no Alt
  keys, and the owner's i3 uses Mod4, so Alt+Up reaches the app.
- **No hardware or network** is involved. Imports in tests use the stub separator with the real
  `calliope-stems` on 127.0.0.1 (Rust) or inside `unshare -rn` (GUI), as today. Tagged sources are
  generated with the local `ffmpeg`.
- **Whole suite**: `npm test` (headless, all crates) and `npm run test:gui` (GUI), both in
  `src/calliope-gui/`.

## 5. Deployment

Only the laptop app changes (`calliope-gui`). It is built with `npm run build:app` as before. The
`calliope-stems` server and its API are unchanged, so nothing is deployed to archserver.

## 6. Manual checks (owner, on the laptop)

1. **Old library**: start the new build on your real repository. Every track shows as before
   (alphabetical, no numbers). Spot-check a few `track.json` files: their modification times are
   unchanged (nothing was rewritten).
2. **Drag with the real mouse and the touchpad**: expand an album and drag its last track above the
   first. While dragging, a line shows where it will land and the row is dimmed. Release: the numbers
   become 1..n. Repeat with the touchpad (press and hold, or a three-finger drag if you use one).
3. **Drags that must do nothing**: drag a track onto another album, onto a band, and outside the
   window, and press Escape in the middle of a drag. Nothing changes. A short click without moving
   still selects the track.
4. **Long album with scrolling**: in a band with several expanded albums, drag a track from the bottom
   to the top of a long album. The column scrolls when you hold the pointer near its edge.
5. **Keys**: focus a track and press Alt+Up / Alt+Down a few times. It moves one place each time and
   keeps the focus. Check that no i3 binding or desktop shortcut takes Alt+Up.
6. **Album order**: put a band's albums in chronological order by dragging their rows. Restart the
   app. The order and each album's track order are kept, and "(no album)" is last.
7. **Delete**: delete a single track with the Delete key, and an album with the toolbar button. Read
   the confirmation (band, album, track count), confirm, and check that the folders are in the
   repository's `trash/`. A band shows the button disabled.
8. **Import a real album folder** whose files carry track numbers (e.g. "3/12"). The tracks appear
   numbered in disc order.
9. **Rename**: fix a misspelt album on one of its tracks (track pane, Edit, change Album, Save). The
   confirmation names both names and the count. Confirm, and all tracks move under the new name with
   their order. Try a band name too.
10. **Older build after this one** (optional): open the repository with the previous build. It still
   loads the reordered tracks (alphabetically), and saving one there keeps its numbers for the new
   build.

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests |
|----------------------|-------|-------|
| A, B, C without positions shown alphabetically without numbers | 7, 11 | `library-tree.test.ts`; `acceptance_library.test.ts`; GUI `library_reorder_tracks` (screenshot) |
| Drag C above A gives "1. C", "2. A", "3. B", also after restart | 2, 4, 8, 10, 11 | `library-order.test.ts`; `LibraryView.test.ts` drag; `acceptance_library.test.ts`; GUI `library_reorder_tracks` (disk + restart shot) |
| Alt+Up on "2. A" gives "1. A", "2. C", "3. B", A keeps the focus | 10, 11 | `LibraryView.test.ts`; `acceptance_library.test.ts`; GUI `library_reorder_tracks` |
| Alt+Up on "1. A" changes nothing | 10 | `library-ops.test.ts`; GUI (`reason=first`) |
| A track dragged onto another album changes nothing | 8, 11 | `library-order.test.ts` (`gapAt` null); `LibraryView.test.ts`; GUI (`reason=outside`) |
| A track added without a position is shown after the positioned ones | 7 | `library-tree.test.ts`; `acceptance_library.test.ts` |
| Drag Gamma above Alpha gives Gamma, Alpha, Beta, also after restart, track order kept | 7, 8, 10, 11 | `library-order.test.ts`; `acceptance_library.test.ts`; GUI `library_reorder_albums` |
| Alt+Down on Alpha gives Gamma, Beta, Alpha | 10, 11 | `acceptance_library.test.ts`; GUI `library_reorder_albums` |
| An album dragged onto another band changes nothing | 8, 11 | `LibraryView.test.ts`; GUI `library_reorder_albums` |
| The "no album" group stays last | 7 | `library-tree.test.ts`; GUI screenshot |
| Directory import with track tags 2, 1, 3 shows "1.", "2.", "3." in tag order | 3, 7 | `media` / `import_job_tests`; `acceptance_existing_track_import.rs`; `library-tree.test.ts` labels |
| The position is read-only in edit mode | 12 | `TrackPane.test.ts`; `acceptance_library.test.ts` |
| "Masterof Puppets" → "Master of Puppets": confirmation with both names and "8 tracks", all 8 moved in order | 2, 4, 9, 10, 12 | `library-rename.test.ts`; `ipc` test; `acceptance_library.test.ts`; GUI `library_rename_album_for_all` |
| "Metalica" → "Metallica" moves both albums, orders kept | 9, 10 | `library-rename.test.ts`; `acceptance_library.test.ts`; GUI `library_rename_album_for_all` |
| Cancelling the rename saves nothing and stays in edit mode | 10, 12 | `TrackPane.test.ts`; `acceptance_library.test.ts`; GUI (`rename cancelled`) |
| With search text, drag and the move keys do not reorder | 10, 11 | `library-ops.test.ts`; `acceptance_library.test.ts`; GUI (`reason=search`) |
| Delete (key or button) on a selected track moves it to the trash | 2, 11 | `LibraryView.test.ts`; `acceptance_library.test.ts`; GUI `library_delete_from_tree` |
| Album of 4: confirmation names band, album, "4 tracks"; all 4 trashed, album gone | 2, 4, 8, 10, 11 | `library-order.test.ts` (text); `LibraryView.test.ts`; `acceptance_library.test.ts`; GUI |
| Cancelling the delete confirmation deletes nothing | 11 | `LibraryView.test.ts`; `acceptance_library.test.ts`; GUI (`delete cancelled`) |
| One folder cannot be moved: the others are deleted, the failure is reported with its reason | 2, 10 | `repository_tests` (conflict + chmod); `library-ops.test.ts`; `acceptance_tracks_repository.rs`; GUI |
| A focused band disables Delete | 10, 11 | `LibraryView.test.ts`; `acceptance_library.test.ts`; GUI (screenshot + `reason=band`) |
| A previous-version track.json loads unchanged with no position | 1, 2 | `track_meta` round trips; `acceptance_tracks_repository.rs` (bytes + revision) |

Requirement 17 (reorders and deletes are logged) is covered by the log lines of 2.4, 2.7, 2.8 and
2.9, asserted in `library-ops.test.ts` and by the GUI tests, which wait on them.

## 8. Open questions (owner 2026-10-20: all recommended answers accepted)

The plan is written with the recommended answers below; please confirm or change them.

1. **(Spec Q1) Renaming to a name that already exists.** Recommended: yes, merge, after the same
   confirmation. The confirmation then says the two will be merged. The renamed tracks or albums keep
   their own order and come after the existing ones. A merged album takes the existing album's place
   among the band's albums.
2. **(Spec Q2) A change of upper/lower case is a rename of all its tracks.** Recommended: yes. The same
   holds for accents ("Beyonce" → "Beyoncé"), since the tree already groups names without regard to
   accents.
3. **(Spec Q3) A "(no album)" track gets an album.** Recommended: yes, only that track moves, without
   a confirmation. It joins at the end of that album (the next free number when the album is
   numbered) and in the album's place. Clearing a non-empty band or album name in the track pane is
   refused with a message, because it would take the track out of its band or album.
4. **(New) A "(no band)" track gets a band.** "(no band)" is not a band, so renaming it for all its
   tracks would make no sense. Recommended: the track's whole album under "(no band)" moves to that
   band, keeping its order, after the usual confirmation (so the album is never split). A "(no band)"
   track without an album moves alone, without a confirmation.
5. **(New) Band and album both changed in one Save.** The two renames have different scopes (the whole
   band, or one album), and combined with merges the result is hard to predict. Recommended: refuse
   with "Change the band or the album, save, then change the other."
6. **(New) How drag and drop works in the Tauri webview on Linux.** HTML5 drag and drop
   (`draggable`, `dragstart`/`drop`) is unreliable in WebKitGTK. WebKit bug 265857 reports that most
   drag events, including `drop`, do not fire unless `dataTransfer.setData` is called, in Epiphany and
   in Tauri. Tauri's own window-level drag-drop handler (file drops) also sits on the same GTK
   machinery, and synthetic xdotool drags through GTK drag and drop are flaky in tests. Recommended:
   **pointer events** (pointerdown / move / up), the same technique the Player waveform already uses
   for its drags. It needs no Tauri config change, behaves the same with mouse and touchpad, and is
   testable with xdotool and jsdom. The cost is that there is no floating "ghost" copy of the row: the
   dragged row is dimmed in place and a line marks the drop gap. Escape cancels, and the column
   scrolls near its edges.
7. **(New) Deleting an album while the search box filters the tree.** The filtered album may show only
   some of its tracks. Recommended: Delete still deletes the whole album, and the confirmation gives the
   full count ("8 tracks"), so nothing hidden is deleted without being counted. (The alternative is to
   disable album Delete while searching.)
8. **(New) An imported file has a track number but no album tag.** Recommended: no position is stored,
   because "(no album)" is not an album and is never ordered (requirement 16).
9. **(New) Where reordering and deleting are available.** Recommended: only in the Library's tree. The
   Editor and Player use the same tree and show the same order and numbers, but read-only, with no
   Delete button. The spec says "in the Library tree", and deleting from the Player or the Editor is
   out of scope.
10. **(New) What the Delete button looks like.** The tree column is 20 rem wide, and a labelled "Delete"
   button next to "Collapse all" and "Expand all" would wrap the toolbar onto a second row.
   Recommended: a trash-icon button on the same row, with the tooltip "Delete the focused track or
   album (Delete)" and the accessible name "Delete". (The alternative is a labelled button on a row of
   its own, which moves the tree down by about 40 px.)
