# Report: Library album and track order, deleting tracks and albums

**Status: DONE.** All acceptance criteria and requirements 1-17 pass. The tester found no defects, and the
reviewer approved with no fix round. The full headless suite and the full GUI suite (8 binaries) pass. Branch
`spec/gui-library-album-order`, base `f45e2da`, not pushed or merged.

## What was built

- **Order stored per track:**
  - `track.json` gains two optional numbers: `position` (the track's place in its album) and
    `album_position` (the album's place in its band, written on every track of that album).
  - There is no schema bump. Older builds already keep unknown fields and write them back.
  - Files without them load unchanged, in alphabetical order, and a bad value reads as "no position".
- **Tree (Library only):**
  - In an album, tracks with a position come first, by number and shown as "3. Title". The tracks without one
    follow, alphabetically.
  - A band's albums work the same way, with no number shown. "(no album)" is always last, and bands stay
    alphabetical.
- **Reordering:**
  - Drag and drop inside the album, or among the band's albums. The dragged row is dimmed and a line marks
    where it will land. Escape cancels, and the column scrolls near its edges.
  - Alt+Up / Alt+Down moves the focused track or album one place, and the focus stays on it.
  - Every move renumbers that album, or that band's albums, as 1..n and saves at once.
  - A drop onto another album or band does nothing.
  - Reordering is off while a search filters the tree and while the track pane is in edit mode.
  - The Editor and Player trees show the new order, read-only.
- **Delete from the tree:**
  - A trash-icon button on the Collapse all / Expand all row, and the Delete key, act on the focused or selected
    track or album, after a confirmation.
  - The album confirmation names the band, the album and the track count; the whole album is deleted even when a
    search hides some of its tracks.
  - Tracks go to the repository's `trash/` folder, as with the track pane's Delete.
  - If some tracks fail, the others are still deleted and the failures are listed with their reasons.
  - Focus moves to a neighbouring item. A band can't be deleted, and the button is disabled on it.
- **Imports** set the position from the file's track-number tag ("3" or "3/12" gives 3), but only when the file
  has an album. They never change the tracks already in the album.
- **Track pane:**
  - A read-only "Position" row.
  - **Renames:** changing Band or Album renames that band or album for all of its tracks. A confirmation names
    the old name, the new name and the track count; No keeps edit mode. The tracks and albums keep their order,
    and a name that already exists merges, with the renamed items after the existing ones.
  - **Moves and refusals:**
    - A "(no album)" track that gets an album joins it alone, at the end, with no confirmation.
    - A "(no band)" album that gets a band moves with all its tracks.
    - Clearing a name, or changing band and album in one save, is refused.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| A, B, C without positions → alphabetical, no numbers | AC1; `library-tree.test.ts` | PASS |
| Drag C above A → "1. C", "2. A", "3. B", also after a restart | AC2; `LibraryView.test.ts`; GUI `library_reorder_tracks` | PASS |
| Alt+Up on "2. A" → "1. A", "2. C", "3. B", focus kept | AC3/AC4 | PASS |
| Alt+Up on "1. A" → nothing changes | AC3/AC4 (`reason=first`) | PASS |
| Track dragged onto another album → nothing changes | AC5/AC9; GUI `library_reorder_tracks` | PASS |
| New unpositioned track → shown after the positioned ones | AC6 | PASS |
| Drag Gamma above Alpha → Gamma, Alpha, Beta after a restart, track order kept | AC7/AC8/AC10; GUI `library_reorder_albums` | PASS |
| Alt+Down on Alpha → Gamma, Beta, Alpha | AC7/AC8/AC10; GUI `library_reorder_albums` | PASS |
| Album dragged onto another band → nothing changes | AC5/AC9; GUI `library_reorder_albums` | PASS |
| "(no album)" stays last | AC7/AC8/AC10; `library-tree.test.ts` | PASS |
| Directory import tagged 2, 1, 3 → numbered in tag order | `ac_directory_import_takes_the_positions_from_the_track_tags`; `batch_saves_the_tag_positions_and_leaves_existing_tracks_alone` | PASS |
| Edit mode → position read-only | AC11; `TrackPane.test.ts` | PASS |
| Album rename for 8 tracks, order kept | AC12-AC14; GUI `library_rename_album_for_all` | PASS |
| Band rename moves both albums, orders kept | AC12-AC14; GUI `library_rename_album_for_all` | PASS |
| Cancelled rename → nothing saved, still in edit mode | AC12-AC14; GUI `library_rename_album_for_all` | PASS |
| Search text → no reorder | AC15; GUI `library_reorder_tracks` (`reason=search`) | PASS |
| Delete a track (key or button) → to the trash, gone from the tree | AC16; GUI `library_delete_from_tree` | PASS |
| Album of 4 → confirmation names band, album, "4 tracks"; all 4 deleted | AC17/AC18; GUI `library_delete_from_tree` | PASS |
| Cancelled delete → nothing deleted | AC17/AC18; GUI `library_delete_from_tree` | PASS |
| One folder can't be moved → the others deleted, the failure reported | AC19; `repository_tests`; GUI `library_delete_from_tree` | PASS |
| Band focused → Delete disabled | AC20; GUI `library_delete_from_tree` | PASS |
| Previous-version `track.json` → loads unchanged, no position | AC21; `old_track_json_files_load_unchanged_with_no_position` | PASS |

The tester also covered your answers and the edge cases:
- **Renames:** merges of albums and bands, case-only renames, a "(no album)" track joining an album, a
  "(no band)" album or a lone track getting a band, and the refusals.
- **Edges:** Alt+Down on the last item, a reorder in edit mode, an album delete during a search, focus after a
  delete, and track-number parsing ("3", "03", "3/12", " 7 ", bad values).

The AC tests are in `src/ui/acceptance_library.test.ts`, and the GUI tests in `tests/gui_library_e2e.rs`.

## Task log

| Step | By | Result |
|---|---|---|
| plan | architect | 15 tasks (2 routine); you accepted all 10 recommended answers |
| tasks 1-5: positions, batch updates and deletes, tag positions, commands | implementer | done; task 5 moved from the local model because of its past `build.rs` breakage |
| tasks 6-10: TS types, tree order, reorder and rename planners, state | implementer | done; 4 existing tests that edited Band/Album to test save errors now edit the Title |
| tasks 11-12: tree UI, track pane | implementer | done; GUI Tab counts updated for the new Position row |
| tasks 13-15: acceptance tests, GUI tests and screenshots, docs | implementer | done; task 15 moved from the local model |
| GUI suite | me + implementer | the existing `edit_and_save` test edits an Album and needed to answer the new rename confirmation; fixed |
| tester | tester | all PASS; 13 tests added; no defects |
| reviewer | reviewer | APPROVED; 4 minor, 2 nits |

The local model was not used in this build. The two routine tasks were edits inside long files that it broke in
the last build.

## Findings (reviewer: fine to ship, left as follow-ups)

1. **A hand-edited name can block a reorder.** A track whose band or album has leading or trailing spaces makes a
   reorder or rename of its album refuse. Such names never come from the app (imports and saves trim them), only
   from a hand-edited `track.json`.
   - The error names the track.
   - Editing and saving that track fixes it.
   - The tester's padded-names test passes, but it uses the fake backend.
   - Suggested fix: check only the fields a reorder changes.
2. **A write that fails halfway** (read-only folder, full disk) during a reorder leaves duplicate numbers in the
   album. The error is shown, and a new reorder fixes it. Nothing tests that case yet.
3. **Escape during a drag:** after Escape, releasing the mouse can still count as a click on the row (it
   expands/collapses or selects it).
4. **Stale position on join:** a "(no album)" track with a leftover hand-edited position keeps it when it joins
   an album.
5. *Nits:* `delete_tracks` doesn't reject duplicate ids (the screen never sends them); when the last track is
   deleted, the keyboard focus goes nowhere.

## Deviations from the plan

- The four frontend tests and the `edit_and_save` GUI test that used Band or Album edits for other checks now edit
  the Title, or answer the rename confirmation. Requirement 11 changed what those edits do.
- `delete_tracks` tests detect root by trying to write, because the crate has no `libc`.
- If a file's `track` tag is invalid, its `tracknumber` tag is not tried.

## Try it

`cd src/calliope-gui && npm run dev:app`, or `npm run build:app` and run `target/release/calliope-gui`. In the
Library, expand an album and drag a track, or focus it and press Alt+Up / Alt+Down. Delete a track or album with
the trash button or the Delete key.

## Deployment

The laptop app only: build with `npm run build:app` and install as before. Nothing changes on archserver. Older
builds still read the reordered `track.json` files (alphabetically) and keep the new fields when they save.

## Manual checks (laptop)

- [ ] **M1, old library.** Start the new build on your real repository. Every track shows as before
  (alphabetical, no numbers), and a few `track.json` modification times are unchanged.
- [ ] **M2, drag with the mouse and the touchpad.** Drag an album's last track above the first: the row dims, a
  line marks the target, and the numbers become 1..n.
- [ ] **M3, drags that do nothing.** Onto another album, onto a band, outside the window, and Escape mid-drag.
  A short click still selects. (Watch for finding 3 after Escape.)
- [ ] **M4, long album.** Drag from the bottom to the top of a long album: the column scrolls near its edge.
- [ ] **M5, keys.** Alt+Up / Alt+Down move one place each and keep the focus. Check that no i3 or desktop
  shortcut takes Alt+Up.
- [ ] **M6, album order.** Drag a band's albums into chronological order and restart. The order and each album's
  track order are kept, and "(no album)" is last.
- [ ] **M7, delete.** Delete a track with the Delete key and an album with the button. The confirmation names
  band, album and count, and the folders end up in `trash/`. The button is disabled on a band.
- [ ] **M8, import a real album folder** whose files have track numbers (e.g. "3/12"): the tracks appear in disc
  order.
- [ ] **M9, rename.** Fix a misspelt album on one of its tracks. The confirmation names both names and the count,
  and all tracks move with their order. Try a band too.
- [ ] **M10 (optional), older build.** The previous build still loads the reordered tracks, and saving one there
  keeps its numbers.

Once you've verified it, tick `gui-library-album-order.md` in the roadmap in `specs/overview.md`.
