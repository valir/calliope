# Report: gui-export-info

Status: **DONE**

Branch `spec/gui-export-info`, base `caa6db3`. Not pushed or merged.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| Never exported, exported to memory 7: RC-5 entry (7, time); Library row | `ac_info_rc5_recorded`; `TrackPane.test.ts`; GUI `export_rc5_flow` | PASS |
| Entry for 7: the Memory box shows 7 | `ac_info_memory_from_record`; `ExportView.test.ts`; GUI `export_rc5_flow` (after restart, nothing typed) | PASS |
| Same base: confirmation names `007_1/007_1.WAV`; after Yes the time is newer, memory still 7 | `ac_info_rc5_reexport_same_memory`; GUI `export_rc5_flow` | PASS |
| Entry 7, exported to 12: entry says 12 | `ac_info_rc5_other_memory` | PASS |
| A holds 7, B exported to 7: A's RC-5 entry cleared, its other entries and fields kept | `ac_info_rc5_clears_previous_holder`, `ac_info_cleared_holder_keeps_revision_fields_and_survives_a_stale_save`; GUI `export_rc5_flow` | PASS |
| MP3 and OGG full paths, kept apart | `ac_info_file_paths`; GUI `export_mp3_and_ogg` | PASS |
| MP3 entry in `D`: the dialog opens in `D` | `ac_info_mp3_dialog_starts_in_recorded_folder`, `ac_info_ogg_dialog_start_and_fallback_to_the_remembered_folder`; GUI `export_mp3_and_ogg` (`from=recorded`) | PASS |
| Cancelled, refused or failed export: metadata unchanged | `ac_info_nothing_recorded_on_cancel_refusal_failure`, `ac_info_cancel_and_failure_leave_the_other_holder_alone` | PASS |
| Library save keeps the info, which can't be edited | `ac_info_kept_by_library_save`; `TrackPane.test.ts`; GUI `export_info_keeps_library_edit` | PASS |
| Apart from the info, metadata and files unchanged | `repo_unchanged_except_exports()` in every successful export test | PASS |
| Never exported: no export rows | `ac_info_never_exported_has_none`; `TrackPane.test.ts` | PASS |
| Info not saved is reported, file still written (req. 8) | `ac_info_not_saved_is_reported`, `ac_info_mp3_not_saved_is_reported_and_the_file_is_written` | PASS |
| Older builds keep the info (constraint) | `ac_info_older_builds_keep_the_info`; `track_meta` unit tests | PASS |

Suites on `b282aa6`: `npm test` passes (now including `cargo fmt --all --check`), and the GUI
suite passes (all 9 binaries).

## Task log

| Step | By | Commit |
|---|---|---|
| plan | architect | 216d8f7 |
| tasks 1-3: record module, exports-free revision, repository record/clear, rules | implementer | 54358f2 |
| style: whole workspace formatted with cargo fmt; `npm test` checks it (owner decision) | me | 95875f0 |
| tasks 4-6: job hook, IPC, headless acceptance tests | implementer | 7c48502 |
| tasks 7-9: frontend state, Export pane, Library rows | implementer | 8944ebf |
| tasks 10-11: GUI tests, docs | implementer | 6f6005c |
| tester: 4 more acceptance tests, no defects | tester | df8198c |
| fix round 1 | implementer | b282aa6 |

The local model handled no task: every task added logic in a new file or edited a long one.

## Review

APPROVED, and APPROVED again after fix round 1. Fix round 1 handled:
- a finished export no longer reloads the pane for another selected track;
- the Export pane rows have a fixed layout, so the GUI test's button positions don't depend on the
  "Last:" text;
- the refill flag is no longer set for a null id;
- a test flake ("Text file busy") is gone. Its cause was a fake `ffprobe` script being rewritten
  while a cancelled job still ran it. Fake tool scripts are now written and renamed.

Remaining minors, left as they are:
- A "Last:" path longer than two lines still makes its row taller. Only the GUI test cares, and
  its temp paths fit in two lines.
- In a rare race (a refill load superseded by a "no track" load), the next load of the same track
  refills the Memory box over a typed value.
- A track with a malformed hand-edited `exports` entry is never treated as an RC-5 holder, so the
  entry stays.
- After an export, the keys of `track.json` are in alphabetical order until the next Library save.
  This is cosmetic, and the owner accepted it.

## Deviations from the plan

- **Formatting:** the whole workspace was reformatted with rustfmt (owner decision). `CLAUDE.md`
  states the rule, and `npm test` fails on unformatted code.
- **Existing tests updated:** whitespace no longer changes a track's revision, so two tests that
  made "changed on disk" by adding whitespace now change a field:
  `save_conflict_changes_nothing` and `gui_library_e2e` `library_delete_from_tree`.
- **Export pane layout:** each "Last:" line sits under its button. The RC-5 line goes under the
  Memory row.

## Follow-ups

- `stretch::tests::tone_keeps_pitch` failed once in the tester's run and passed on reruns. This
  feature doesn't touch that code. Watch for it.
- With `--features e2e-hooks`, clippy reports dead code for `SPEED_PLUS` in
  `tests/gui_player_e2e.rs`. This predates the feature, and `npm test` doesn't build with that
  feature.

## How to try it

Laptop: pull, `npm run build:app`, install as before. `track.json` gains an optional `exports`
object, with no schema bump. Older builds load it and keep it.

## Manual checks (laptop, real RC-5)

- [ ] **Record:** export a track to RC-5 memory 7. The Library shows "RC-5: memory 7 (<today>
  <local time>)".
- [ ] **Re-export:** select the same track in Export. The Memory box shows 7, with "Last: memory 7,
  <today>". "to BOSS RC-5" asks to replace `.../ROLAND/WAVE/007_1/007_1.WAV`. After Yes, memory 7
  on the unit plays the reworked track.
- [ ] **One track per memory:** export another track to memory 7. The first track's RC-5 row is
  gone, and the second shows memory 7.
- [ ] **MP3 and OGG:** export as MP3, then press "MP3 file" again. The dialog opens in that folder,
  and Yes replaces the file. Do the same for OGG. The Library shows both rows with full paths.
- [ ] **Edit during an export:** in the Library, edit a track's year. Switch to Export and export
  it as MP3, then come back and Save. There is no "changed on disk", and the MP3 row stays.
- [ ] **Older build (optional):** open the repository with a build from before this feature. Edit
  and save an exported track: `track.json` keeps `exports`.

Roadmap: `gui-export-info.md` is already ticked in `specs/overview.md`. Untick it until the manual
checks pass if you want the roadmap to show verified features only.
