# Report: gui-export-rc5-quick

Status: **DONE**

Branch `spec/gui-export-rc5-quick`, base `ecea1ce`. Not pushed or merged.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| Export view just opened: the dialog opens at the remembered folder | `ac_quick_first_export_asks_at_the_remembered_folder`; `ExportView.test.ts`; GUI `export_rc5_flow` | PASS |
| After a success to `B`, another track: no dialog, written under `B/ROLAND/WAVE/` | `ac_quick_next_export_skips_the_dialog`, `ac_quick_mp3_between_rc5_exports_keeps_the_skip`; GUI `export_rc5_flow`, `export_rc5_quick` | PASS |
| Memory file exists: the confirmation names it, "No" writes nothing | `ac_quick_overwrite_still_asks_and_no_writes_nothing`; GUI `export_rc5_flow` (No, then Yes, both without the dialog) | PASS |
| `B/ROLAND` removed: the dialog opens | `ac_quick_rc5_gone_opens_the_dialog`, `ac_quick_roland_file_and_removal_between_prepare_and_start` | PASS |
| Player and back to Export: the next RC-5 export asks | `ExportView.test.ts`; GUI `export_rc5_quick` | PASS |
| The row shows `B`; Change... opens the dialog and its folder is used | `ac_quick_change_folder_is_used`, `ac_quick_change_cancelled_or_refused_keeps_the_folder`; `ExportView.test.ts`; GUI `export_rc5_quick` | PASS |
| Failed or cancelled export: the next one asks | `ac_quick_failed_or_cancelled_sets_nothing`; `ExportView.test.ts` | PASS |
| MP3 succeeded: MP3 again opens the dialog | `ac_quick_mp3_always_asks` | PASS |
| 1280x800 and 1024x640: nothing overflows | GUI `export_rc5_quick`: `overflow=0 hscroll=0` at both sizes | PASS |

The tester also checked:
- the window naming a folder Rust doesn't hold, which opens the dialog and writes nothing;
- a symlinked drive later pointed into the repository, which is refused;
- `ROLAND` being a file instead of a folder;
- the drive removed between choosing and writing;
- Change... and Export both refused while an export runs;
- a cancelled or refused Change..., which keeps the previous folder.

Suites on `8d6baff`: `npm test` passes on the final commit. In the full GUI suite every Export, Editor, Player and
Library test passed; four tests elsewhere hit the known app-startup timeout and passed when rerun alone.

## Task log

| Step | By | Commit |
|---|---|---|
| plan | architect | 0836cda |
| tasks 1-3: remembered folder, skip rule, export command skip, Change... command | implementer | 5e70833 |
| task 4: register the command | me (local result discarded) | 3e30d24 |
| tasks 5-7: acceptance tests, window state, "To:" row and Change... | implementer | d5cf97f |
| task 8: GUI tests, `docs/ui.md` | implementer | a0dd7e9 |
| task 9: README | local model (indentation fixed by me) | 8d6baff |
| tester: 6 more acceptance tests, no defects | tester | a04de7b |

**Local model:**
- **Task 9:** wrote the README bullet correctly but indented it one level too deep; I fixed the indentation.
- **Task 4:** replaced the `prepare_export` registration lines instead of adding new lines after them. The registration
  check still passed, because all three lists lost the same entry, but the export command would have been gone. I
  discarded it and made the three insertions myself.

## Review

APPROVED, no blockers or majors. The reviewer confirmed:
- Rust only skips the dialog for the folder it remembers itself, never one the window names.
- Every existing refusal and the overwrite confirmation still apply.
- The folder is checked again when the export starts.

Minor, no fix needed:
- a `ROLAND` symlink inside the picked folder isn't resolved, the same as the normal dialog path;
- a stale "done" event could make the window offer a skip that Rust then declines. That falls back to the dialog.

## Deviations from the plan

- **MP3 and OGG buttons:** they moved down by 67 / 39 px in the GUI tests (the plan estimated 36). The old coordinates
  only sat somewhere inside the buttons; the new ones were measured.
- **Layout:** measured at `overflow=0` (the plan allowed up to 14).

## Follow-ups

- **Disk space:** the first full GUI run failed with "No space left on device".
  - I deleted the compiler's incremental cache (`target/debug/incremental`, 21 GB, rebuilt by cargo as needed).
  - `target/debug` still holds about 35 GB: `cargo clean` frees it, at the cost of one full rebuild.
- **Startup timeouts:** the full parallel GUI run still hits app-startup timeouts in a few tests now and then.

## How to try it

Laptop: pull, `npm run build:app` in `src/calliope-gui/`, install as before. No settings or file format change, no
server change.

## Manual checks (laptop, real RC-5)

- [ ] **First export asks:** mount the RC-5, start Calliope, export track 1 to memory 1: the folder dialog opens at the
  RC-5 drive you used last. Pick it; the RC-5 row now shows "To: /run/media/<you>/BOSS_RC-5".
- [ ] **Batch:** track 2, memory 2, "to BOSS RC-5": no dialog, "Written to .../ROLAND/WAVE/002_1/002_1.WAV". Again for
  track 3, memory 3.
- [ ] **Overwrite:** track 1, memory 1: the question names `001_1.WAV`. No: nothing is written. Press again (still no
  folder dialog) and answer Yes.
- [ ] **Unplugged:**
  1. Eject and unplug the RC-5, then press "to BOSS RC-5": the folder dialog opens.
  2. Cancel it: the "To:" line is gone.
  3. Plug it back in and export: the dialog asks once, then the next exports skip it.
- [ ] **Change...:**
  1. Pick a folder without `ROLAND`: "Folder not changed: No ROLAND folder in ...".
  2. Pick the RC-5: the "To:" line shows it.
- [ ] **Leaving the view:** Player (Alt+5) and back (Alt+6): the "To:" line is empty and the next RC-5 export asks.
- [ ] **MP3:** export an MP3 twice: the dialog opens both times.
- [ ] **On the RC-5:** play memories 1-3; each is the right track.
- [ ] **Look:** at your usual size and about 1024x640, the "To:" line and Change... sit under the RC-5 button without
  overlapping anything, and the MP3 / OGG buttons don't move after the first export.

Roadmap: add/tick `gui-export-rc5-quick.md` in `specs/overview.md` once the manual checks pass.
