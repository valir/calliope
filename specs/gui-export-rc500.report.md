# Report: gui-export-rc500

Status: **DONE**

Branch `spec/gui-export-rc500`, base `30e8754` (spec, plan and architecture committed by the owner in `95e630c`). Not
pushed or merged.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| Memory 7, Track 1: `B/ROLAND/WAVE/007_1/007_1.WAV` in the RC-5's WAV format | `ac_rc500_memory_7_track_1` (bytes equal to the RC-5 export), `ac_rc500_memory_boundaries_and_both_tracks_byte_identical_neighbours` | PASS |
| Memory 7, Track 2: `007_2/007_2.WAV`, `007_1/007_1.WAV` byte-identical afterwards | `ac_rc500_track_2_leaves_track_1_alone`, the boundaries test (both directions); GUI `export_rc500_flow` | PASS |
| `007_2.WAV` exists: the confirmation names it, "No" writes nothing | `ac_rc500_overwrite_needs_confirmation`; GUI `export_rc500_flow` | PASS |
| A folder without `ROLAND` is refused | `ac_rc500_without_roland`, `ac_rc500_roland_symlink_and_roland_file` | PASS |
| After an RC-500 success, the next RC-500 export skips the dialog; an RC-5 export still asks | `ac_rc500_quick_is_its_own`, `ac_rc500_and_rc5_quick_folders_are_independent_both_ways`; GUI `export_rc500_flow` | PASS |
| After a restart each dialog opens at its own folder | `ac_rc500_and_rc5_folders_survive_a_restart`; GUI `export_rc500_flow` (restart step) | PASS |
| One track per memory and track; track 1 and RC-5 records kept | `ac_rc500_one_track_per_memory_and_track` | PASS |
| Library "RC-500 memory 7, track 2 (...)"; Export view shows memory 7 and Track 2 | `ac_rc500_info_follows_the_record`; `TrackPane.test.ts`, `ExportView.test.ts`, `export-format.test.ts`; GUI `export_rc500_flow` | PASS |
| Slow switch: the RC-500 file has the slow count-in | `ac_rc500_slow_count_in` | PASS |
| 1280x800: no scrollbar; 1024x640: no horizontal scroll | GUI `export_view_buttons`, `export_rc500_flow`: overflow 0 / hscroll 0 at 1280x800, hscroll 0 at 1024x640 | PASS |

The tester also checked:
- memories 1, 99 and " 7 "; bad memories (0, 100, "abc") and tracks (0, 3) refused before any dialog;
- `007_2` existing as a file: the export fails and nothing else is written, recorded or remembered;
- a hand-edited `exports.rc500` (memory 200, track 5, track "two", no track): ignored, suggestions stay valid;
- the RC-5 and RC-500 quick folders never stand in for each other;
- RC-5, MP3 and OGG unchanged (every existing export test passes).

Suites on `7833730`: `npm test` passes. The whole GUI suite passed on the first try on `d5041b8` (the tester only
added headless tests after that).

## Task log

| Step | By | Commit |
|---|---|---|
| spec, plan, `docs/architecture.md` | architect (committed by the owner) | 95e630c |
| tasks 1-2: `exports.rc500` record, settings, dialog kind | implementer | 6d34846 |
| tasks 3-4: RC-500 export kind and paths, quick folder per kind, export commands | implementer | 9d14fcc |
| task 5: register `choose_rc500_folder` | local model | 8287ae1 |
| tasks 6-8: acceptance tests, window state and texts, Library row, two-column pane | implementer | baa85a2 |
| task 9: README | local model | 0a580bd |
| task 10: GUI tests, geometry guard, `docs/ui.md` | implementer | 47ad0cb |
| task 10: race in `export_rc500_flow` fixed | implementer | d5041b8 |
| tester: 5 more acceptance tests, no defects | tester (one clippy nit fixed by me) | 7833730 |

The local model did tasks 5 and 9 correctly, with no changes needed.

## Review

APPROVED, no blockers or majors; no fix round. The reviewer confirmed:
- every RC-5-only check now covers both loopers, and nothing RC-5-specific leaked into the RC-500;
- only `WAVE` and `0NN_<t>` are ever created, and removed again on failure or cancel;
- the track (1 or 2) and the memory are checked in Rust, not trusted from the window;
- Rust skips the dialog only for its own remembered folder of the same kind;
- RC-5 texts and log lines are unchanged byte for byte.

**The ignored click** (found while fixing the GUI test): right after an RC-500 export finished, the test's click on
the RC-5 button got lost. The reviewer and the tester both found that a real user can't hit this: while the export
finishes, the buttons are visibly greyed out, and nothing else blocks them. The tester couldn't make it happen again
without the test's wait.

Minor, left as is:
- After any export, both Memory boxes (and the Track choice) refill from the track's records, so an RC-500 memory you
  typed but didn't use is replaced after an RC-5 export (and the other way round).
- The RC-500 Memory box is labelled for screen readers differently from the RC-5's (works the same).
- Two small code tidy-ups (a type that could be stricter, repeated log-text branches).

## Deviations from the plan

- **Layout:** none of the plan's fallback squeezes were needed: overflow 0 at 1280x800 with every line shown.
- **Restart step of the GUI test:** after leaving the Export view the RC-500 export opens the dialog again (as the
  spec says), so the test answers it instead of expecting a skip.
- `PlanInput` already had a `track` field, so the RC-500 track is `rc500_track` there.
- The README's first sentence now runs one line longer than the rest of the paragraph.

## Follow-ups

- Refill only the box of the device that was exported (the first minor above), if it bothers you.
- The RC-500's memory settings file (tempo, loop mode) isn't written, by design; see manual check 2.
- **Disk space:** `/` has about 16 GB free; `cargo clean` frees the build output in `target/` (about 35 GB) at the
  cost of a full rebuild.

## How to try it

Laptop: pull, `npm run build:app` in `src/calliope-gui/`, install as before. `settings.json` gains `rc500_dir` /
`rc500_memory`, `track.json` the optional `exports.rc500`; older builds ignore both. No server change.

## Manual checks (laptop, real RC-500 and RC-5)

- [ ] **Track 1:** mount the RC-500, pick a track with a tempo, Memory 7, Track 1, "to BOSS RC-500", choose the drive
  root (the folder with `ROLAND`). "Written to .../ROLAND/WAVE/007_1/007_1.WAV. Eject the RC-500 ..." appears. Eject;
  on the pedal, memory 7, track 1 plays the silence, the count-in, then the track.
- [ ] **Track 2 keeps track 1:** mount again, another track, Memory 7, Track 2, export. Staying in the Export view, no
  dialog opens ("To: <drive>"). Eject: on memory 7 both tracks play their own backing. If the pedal shows track 2 as
  empty or refuses it, tell me (the memory's settings file is not written).
- [ ] **Overwrite:** memory 7, Track 2 again: the question names `.../007_2/007_2.WAV`. No: nothing changes. Yes on a
  second try: the new file plays.
- [ ] **Remembered:** select the first track again: the RC-500 box shows 7 with Track 1, and "Last: memory 7, track 1,
  <date>" is under the button; the Library has "RC-500 memory 7, track 1 (<date time>)". A track never exported to the
  RC-500 shows the last memory plus one, with Track 1.
- [ ] **Separate drives:** restart Calliope: "to BOSS RC-500" opens at the RC-500 drive, "to BOSS RC-5" at the RC-5's.
- [ ] **RC-5 regression:** export to the RC-5, memory 3: `ROLAND/WAVE/003_1/003_1.WAV` as before, plays on the RC-5,
  its "To:" line and Change... work. An RC-5 export to memory 7 leaves the RC-500 "memory 7" Library row alone.
- [ ] **Slow count-in:** with the switch on, an RC-500 export has the slow count-in on the pedal.
- [ ] **Look:** at your usual size the RC-5 and RC-500 blocks sit side by side, MP3 / OGG under them, no scrollbar; at
  about 1024x640 they stack and only scroll vertically. The RC-5's Change... is now on its Memory row.

Roadmap: tick `gui-export-rc500.md` in `specs/overview.md` once the manual checks pass.
