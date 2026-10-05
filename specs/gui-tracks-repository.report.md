# Report: Track Repository (Library view)

**Status: DONE.** All 19 acceptance criteria pass. The reviewer approved after 1 fix round.

- Branch: `spec/gui-tracks-repository` (base `f7894d9`), not pushed or merged
- Plan: `specs/gui-tracks-repository.plan.md`, log: `specs/gui-tracks-repository.log.md`

## Acceptance criteria

| Criterion | Tests | Result |
|---|---|---|
| Tracks listed, collapsed at start | vitest `acceptance_library` + `LibraryView`; e2e `library_starts_collapsed` | PASS |
| Three-level tree, sorted case-insensitively | vitest, `library-tree.test.ts` | PASS |
| Expand all / Collapse all | vitest; e2e | PASS |
| Search per keystroke, case-insensitive, fuzzy (`nght`, `brn`, `cafe`) | `fuzzy.test.ts`, vitest; e2e `search_filters` | PASS |
| Empty list / no selection: fields visible, inactive | vitest (AC6, AC7) | PASS |
| Selection: metadata shown, buttons except Save active, bar below the fields | vitest (AC8) | PASS |
| Edit mode: fields editable, ID read-only, buttons switch | vitest (AC9) | PASS |
| Save writes to disk, `modified` updated, back to read-only | Rust `ac10_*`; vitest; e2e `edit_and_save` (disk check) | PASS |
| Tablature list / only Add when there are none / Update-Export-Remove when one is selected | vitest (AC11-AC13) | PASS |
| Add: dialog filtered to tablature files, picked file added without its path, selected | IPC + vitest (AC14, AC15); e2e `add_opens_real_file_dialog` (real GTK dialog, "Tablature files" filter) | PASS |
| Save after Add copies the file, lists it, updates the time; the original is untouched | Rust `ac16_*`; e2e `tablature_add_and_remove` | PASS |
| Remove: confirmation; Yes removes the row only; Save moves the file to trash | vitest (AC17-AC19); Rust `ac19_*`; e2e | PASS |
| Req 1-13 (fields, default/configurable root, layout, ids, two columns, …) | `req*` acceptance tests; e2e `settings_change_root`, `library_min_size` | PASS |

Suites:
- Headless (`npm test`, display unset):
  - vitest: 182 passed
  - cargo: 92 unit + 152 acceptance (0 ignored) + 14 + 13 + 10 + 3 passed
  - clippy: clean
- GUI tests (`DISPLAY=:1 npm run test:gui`): 5 foundation e2e + 8 Library e2e + 1 smoke, all green. They were run 4+ times, including twice after the fix round.

## Your decisions applied
- **Trash:** deleted tracks and removed or replaced tablatures go to `<root>/trash/`, which Calliope never empties.
- **Removal confirmation:** `Delete tablature "<name>"?`.
- **Cancel:** Cancel or Escape leaves edit mode, asking "Discard your changes?" when there are unsaved changes.
- **No import** in this feature. Try it with the sample repository `tests/fixtures/library-sample`.
- **Default you didn't object to:** track folders are named by a UUIDv7 ID, in a flat `tracks/<id>/`.

## Data safety (what is guaranteed and tested)
- Calliope never deletes or overwrites your files. Removed or replaced files and deleted tracks are moved into `trash/`. A deleted track becomes `trash/<stamp>-<id>/`, and one save's tablatures go to `trash/<stamp>-<id>-tablatures/`.
- **How a save works:**
  1. Check that `track.json` hasn't changed on disk (content hash).
  2. Copy new files in without ever overwriting anything.
  3. Write `track.json` atomically, using a unique temp name.
  4. Move the old files to the trash.

  A failure before step 3 rolls back only Calliope's own copies.
- **Hand edits:** if `track.json` is edited by hand (or by another instance) while you edit, Save refuses. Your draft is kept, and Cancel reloads.
- **Hand-made files load leniently.** Extra spaces and other timestamp formats are fine, and only Calliope's own writes are strict. Tracks from a newer schema version, or with a wrong type, are listed as problems and never rewritten.
- **Probed by the tester, all held:**
  - path traversal in IDs and names
  - forged or mixed-up tokens
  - symlinks: the track folder, `track.json`, tablatures and `trash/`
  - read-only folders
  - name and case clashes
  - stale `.part` and temp files
  - export into the repository
  - very long multibyte names
- **Accepted limits** (documented in `docs/architecture.md`):
  - There is no lock across app instances, only the revision check and unique temp names.
  - A crash in the middle of a same-name replace can leave the new version as `.<name>.part` and the old one in the trash. Nothing is lost, and the track shows that file as missing.

## Security
- The frontend never sends file paths. Rust opens the native dialogs and returns opaque, typed tokens. IDs and names are validated and resolved only inside `<root>/tracks/<id>/`.
- The first Tauri permission files are `capabilities/main.json` and the app's command list in `build.rs`. They grant only the app's 14 commands to window `main`, with no core, dialog or fs permissions. A test keeps the handler list, `build.rs` and the capability file in sync.
- The test-only scripted dialog (`e2e-hooks` feature + `CALLIOPE_E2E_DIALOG_ANSWERS`) is compiled only with the feature. A release build with it fails: "the e2e-hooks feature must never be in a release build".
- The production CSP is unchanged.

## Task log

| Task | Done by | Notes |
|---|---|---|
| 1. fsutil | implementer | trash layout settled by orchestrator |
| 2. track_meta | implementer | orchestrator: lenient read / strict write |
| 3. repository (27 tests) | implementer | orchestrator audited every remove and rename |
| 4. sample fixture | implementer | invented content only |
| 5. settings `repository_root` | implementer | |
| 6. picker + release guard | implementer | |
| 7. IPC, ACL, capability | implementer | |
| 8. `ipc.ts` | implementer | the local model failed (output limit) |
| 9. fuzzy + tree | implementer | |
| 10. edit model + button rules | implementer | |
| 11. textarea, alert-dialog, ConfirmDialog | implementer | |
| 12. Library left column | implementer | |
| 13. Track pane + Tablature panel | implementer | |
| 14. Settings card | implementer | |
| fix | implementer | first run: the default root was never created (found by the task 14 implementer) |
| 15. Library e2e (8 tests, disk checks) | implementer | |
| 16. visual review | implementer | 5 small visual fixes (disabled buttons, edit-mode lock cue, …) |
| 17. licence record | implementer | |
| 18. README | orchestrator | the local model failed (output limit); exact text taken from the plan |
| fix round 1 | implementer | 10 items: the reviewer's major plus data-safety findings (below) |

The local model failed both routine tasks (8 and 18) on its 32k output limit. Over the last two features it has succeeded only on short files with exact content. Consider marking nothing as routine until the local setup changes, for example by raising `CLAUDE_CODE_MAX_OUTPUT_TOKENS` or using a stronger local model.

## Review
- **First review:** CHANGES REQUIRED. The major was that the Inter OFL-1.1 licence text wasn't shipped. The tester found 5 data-safety defects, none of which loses data in the normal flow.
- **Fix round 1 (all done):**
  - Ships `dist/licenses/Inter-OFL-1.1.txt`, with a test.
  - Caps export folder names.
  - `copy_to_part` never deletes a `.part` it didn't create.
  - The trash moves a symlink itself, not its target, and refuses a `trash/` that is a symlink.
  - Unique temp names.
  - Export refused anywhere inside the repository folder.
  - A save conflict keeps your draft.
  - The real-dialog test uses a temporary HOME.
  - The Tablature panel is pinned and visible at 1280x800.
- **Re-review:** APPROVED, with 0 blockers and 0 majors. All 19 acceptance criteria still pass, plus 4 new data-safety probes.

Remaining minors and follow-ups:
- `tests/common/mod.rs` duplicates the older harness in `tests/gui_e2e.rs`; migrate it so they don't drift.
- Unused-tablature tokens never expire (harmless, in memory).
- `validate_file_name` doesn't reject Windows reserved names (`CON.gp5`). Add that when Windows is in scope.
- After a conflict, Save stays enabled; pressing it just conflicts again.
- The `dead_code` allows on `NewTrack`, `create_track`, `new_id` and `TablatureTrash::dir` should go when the import feature lands.
- A one-off empty white box appeared once next to the nav on hover during the visual review. It's probably a native webkit tooltip, and it wasn't reproduced.
- Read-only fields keep their box in view mode while editable ones are borderless. A design call for you.

## How to try it
```sh
npm ci && npm run build:app && target/release/calliope-gui
cp -r tests/fixtures/library-sample ~/calliope-demo   # then Settings > Track repository > Choose folder
DISPLAY=:1 npm run test:gui                           # GUI e2e; screenshots in target/gui-shots/library-*.png
```
Keys: Ctrl+F search, Ctrl+E edit, Ctrl+S save, Escape cancel editing.

## Manual checks (your desktop, real data)
- [ ] Fresh build: the Library says "No tracks in the repository yet."; `~/.local/share/calliope/` has `calliope-repository.json` and `tracks/`
- [ ] Copy the fixture to `~/calliope-demo` and choose it in Settings; you see 4 collapsed bands in order
- [ ] Expand all / Collapse all; `nght`, `brn`, `cafe` narrow the list with each key; Clear restores it
- [ ] Edit "Slow Burn" → change Album → Save; `track.json` shows the change and Modified is now
- [ ] Add a real `.gp5`: the dialog offers only tablatures; after Save it's copied in and your original is untouched
- [ ] Remove it: the confirmation names the file; after Save it's in `trash/`
- [ ] Export the track to `/tmp`; export a tablature and open it in TuxGuitar
- [ ] Delete "Lanterns": moved to `trash/`
- [ ] Hand-edit `track.json` while the track is in edit mode, then Save: refused, with your draft and the hand edit both kept
- [ ] At 1024x640 the action bar and tablature panel are usable; readable from 1-2 m in a dim room
- [ ] Settings → Use default; then remove `~/calliope-demo`

No deployment steps: local build only.

Once you've verified it, tick **gui-tracks-repository** in the roadmap of `specs/overview.md`.
