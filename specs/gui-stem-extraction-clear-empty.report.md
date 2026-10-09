# Report: drop silent stems at import (increment to gui-stem-extraction)

**Status: DONE.** Requirement 8 and its acceptance criterion pass, using your threshold: a stem
is empty when its peak is below -50 dBFS. The reviewer approved with no fix round, and the
tester found no defects. Nothing touched archserver, the real model or a sound device.

- Branch: `spec/gui-stem-extraction-clear-empty` (base `64e3ec4`), not pushed or merged
- Plan: `specs/gui-stem-extraction-clear-empty.plan.md`, log: `specs/gui-stem-extraction-clear-empty.log.md`
- Nothing to deploy: only the app changed; the `calliope-stems` server is untouched

## Acceptance criteria

| Criterion / decision | Tests | Result |
|---|---|---|
| After extraction, empty stems are dropped (peak below -50 dBFS) | `import_job_tests` `silent_stems_are_dropped_from_the_saved_track`; `acceptance_silent_stems` `sparse_import_keeps_only_audible_stems_in_server_order`; GUI `import_drops_silent_stems` | PASS |
| Threshold boundary: 16-bit 103 dropped / 104 kept; 24-bit 26527 / 26528; 8- and 32-bit; any channel | `stem_audio` unit tests; `acceptance_silent_stems` boundary tests | PASS |
| A short loud passage in a long quiet stem is kept | `a_short_loud_passage_in_a_long_quiet_stem_is_kept` | PASS |
| `track.json` and `stems/` list only the kept stems | import job, acceptance and GUI tests | PASS |
| All stems silent: the import fails with "Every stem is silent (below -50 dBFS), so no track was saved"; nothing created, retry works | `all_silent_import_fails_cleanly_with_the_exact_message`, `after_an_all_silent_failure_the_user_can_extract_again_and_succeed` | PASS |
| A stem that can't be decoded is kept | `boundary_and_undecodable_stems_through_a_whole_import` | PASS |
| "Keep the original mix" still works | `sparse_import_with_keep_original_still_adopts_the_full_mix` | PASS |
| Existing tracks are not re-measured or rewritten | `existing_tracks_are_not_rewritten_or_remeasured` | PASS |
| The user sees what was dropped ("Dropped silent stems: piano, other."), also after returning to Import | vitest `ImportView`, `acceptance_silent_stems.test.ts`; GUI screenshot `target/gui-shots/import-done-dropped.png` | PASS |
| A reduced track opens in the Editor with one lane per kept stem | `the_reduced_track_opens_in_the_editor_with_four_lanes` | PASS |
| Data safety: symlinks, cancel, crash between stems | `discard_stem_part_*`, `cancel_while_the_check_runs_leaves_nothing`, `a_crash_between_stems_*` | PASS |

Suites:
- **Headless** (`npm test`, display unset): vitest 376 passed; Rust workspace 0 failed; clippy clean.
- **GUI** (`DISPLAY=:1 npm run test:gui`): all suites green, run twice, including the new
  `import_drops_silent_stems` (gui_import_e2e 14 passed).

## Design
- **Measure:** the stem's sample peak, across all channels, compared with
  `SILENT_STEM_DBFS = -50.0` (`src/calliope-gui/src/stem_audio.rs`). Peak was chosen over an average so that an instrument
  that plays only briefly is never dropped. The decision is integer-only, and decoding stops at
  the first loud sample, so audible stems cost almost nothing. A 15-minute silent stereo stem
  takes about 0.5 s. It uses the existing `claxon`, with no new dependencies.
- **Where:** in the app's import job, right after each stem is downloaded into staging. A silent
  stem's `.part` file is removed (only a real file, never through a symlink). The track still
  appears through one atomic rename.
- **Logs:**
  - `calliope: import job=<id> stem=<name> dropped peak_dbfs=<x> threshold_dbfs=-50` per dropped stem;
  - a `stems kept=… dropped=…` summary;
  - `dropped=<a,b|none>` at the end of the UI's `saved id=` line.
- `docs/architecture.md` has the decision-log entry, including the alternatives the architect rejected.

## Task log

| Task | Done by |
|---|---|
| 1. Quiet stem fixtures | local model (edits); the orchestrator ran the generator script, which the local model isn't allowed to run |
| 2. Stub separator modes `sparse`, `silent` | local model; the orchestrator fixed a quoting bug (file names would have had literal quotes) and the comment |
| 3. The measure | implementer (corrected the plan's 24-bit boundary by one) |
| 4. `Staging::discard_stem_part` | implementer |
| 5. Import job | implementer |
| 6. Import page and log line | implementer |
| 7. GUI test | implementer |
| tester: 24 acceptance checks, all pass | tester |
| reviewer: APPROVED | reviewer |

The local model handled both routine tasks, but neither result could go in unchanged: it can't
run scripts, and its one non-trivial line of bash was wrong.

## Remaining minors (optional)
- Cancel during the silence check of one stem waits for that check to finish (about 0.5 s
  worst case measured).
- The `eprintln` log lines with the dropped peaks are not asserted by a test (stderr isn't
  captured in the headless tests).
- Your spec's criterion still says "all zeroes"; you may want it to say "below -50 dBFS".

## Also on this branch
- `714b092`: the GUI README now lists `pipewire-alsa` as needed to play on PipeWire.

## How to try it
```sh
cd src/calliope-gui && npm run build:app && ../../target/release/calliope-gui 2>&1 | grep calliope
```
Import a song as usual; the finished page names any dropped stems.

## Manual checks (laptop + archserver)
- [ ] Import a real song with no piano: "Import finished" lists the dropped stems, and the track
  folder has only the kept ones
- [ ] Read the dropped peaks: `grep "dropped peak_dbfs"` in the terminal output. If a stem you'd
  call empty was kept (leakage above -50 dBFS), note the song; `SILENT_STEM_DBFS` can be tuned
- [ ] A song with a short, quiet piano part (an intro only) keeps its piano stem
- [ ] The reduced track opens in the Editor with one lane per stem, Mix Play works, Save makes `backings/backing.flac`
- [ ] With "Keep the original mix" on, `original.flac` is still there next to the reduced stems
- [ ] Existing stem tracks still show 6 stems, and their `track.json` is unchanged (`md5sum` before and after)

Once you've verified it, update **gui-stem-extraction** in the roadmap of `specs/overview.md` as you see fit.
