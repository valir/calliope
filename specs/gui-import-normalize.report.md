# Report: gui-import-normalize

Status: **DONE**

Branch `spec/gui-import-normalize`, base `0d2f6ca`. Not pushed or merged.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| A -24 LUFS file: the mix sent measures -14.0 ± 0.1 LUFS, the line says "gain +10.0 dB" | `ac1_quiet_file_is_sent_at_minus_14`; `loudness-format.test.ts`; GUI `import_normalize_result_shown` | PASS |
| Gain capped by the peak: true peak at most -1 dBTP, below -14 LUFS, "limited" line | `ac2_peaks_limit_the_gain`, `qa_loud_and_full_scale_sources_never_exceed_full_scale` | PASS |
| Already at -14.0 LUFS: the mix sent is sample-identical, "not changed" | `ac3_a_file_at_the_target_is_sent_unchanged` | PASS |
| Silence or under 0.4 s: imported as converted, "Too quiet or too short" | `ac4_silence_and_short_files_are_imported_as_is` | PASS |
| Keep original: `original.flac` equals the mix sent | `ac5_the_kept_original_is_the_mix_sent` | PASS |
| URL, video and directory: each file normalized before extraction, summary per file | `ac6_url_video_and_directory_are_normalized_first`; unit batch test; GUI `import_directory_normalize_summary` | PASS |
| Cancel during "Normalizing": nothing saved, temporary files removed | `ac7_cancel_while_normalizing_leaves_nothing`; unit `cancel_leaves_nothing_behind` (measuring and writing); GUI `import_normalize_cancel` | PASS |
| A resumed import is not normalized again (same bytes) | `ac8_a_resumed_import_is_not_normalized_again` | PASS |

The tester also checked:
- mono 48 kHz, 22.05 kHz and 96 kHz 24-bit sources (converted first, then normalized, sent as 24-bit);
- loud and clipped masters: negative gain, no sample beyond full scale;
- crash leftovers and stale or hand-edited `normalize.json`: the step is redone with identical bytes;
- a failing file in a directory import doesn't stop the others;
- a failure in the step is shown as a preparation failure and leaves nothing behind (unit test).

Suites on `c68ce8b`: `npm test` passes. In the whole GUI suite (on `8d38b6f`, before the tester's headless tests)
every test passed except two that hit the known app-startup timeout; both passed when rerun alone, twice.

## Task log

| Step | By | Commit |
|---|---|---|
| plan | architect | 1f7054c |
| task 1: the normalize step, marker and resume | implementer | 2f6b368 |
| task 2: the fake stem server keeps its inputs | local model (executable bit restored by me) | 1e97fcc |
| task 3: the Normalizing phase in every import | implementer | acf4098 |
| tasks 4-6: texts and state, the import screens, acceptance tests | implementer | e017bf4 |
| task 7: GUI tests, `docs/ui.md` | implementer | 663901c |
| task 8: README | local model (a dropped parenthesis fixed by me) | 8d38b6f |
| tester: 5 more acceptance tests, no defects; stray helper script removed | tester, me | c68ce8b |

**Local model:** both tasks had the right content but a slip each:
- task 2: it removed the executable bit of the script it edited;
- task 8: it dropped a `)` and left its own helper script `edit_readme.py` at the repository root, which my commit
  picked up (the reviewer spotted it; removed in `c68ce8b`).

## Review

APPROVED, no blockers or majors; no fix round. The reviewer confirmed:
- the step reuses the backing rules and meter unchanged; the converted `audio.flac` is never rewritten;
- every source goes through the step, and the stems and the kept original use its result;
- resume checks the marker against both files, so a song can't be normalized twice;
- cancel and failure clean up like the other preparation steps;
- both passes stream the audio, so long songs don't sit in memory.

Minor, left as is:
- the test pace setting is compiled into release builds (only its switch is test-only), like the export's;
- an unchanged song's file header is opened once more just for the log line.

## Deviations from the plan

- **1024x640:** the Loudness row is only visible after scrolling the import pane, which already scrolled at that
  size.
- For a local or directory file there's no separate "Preparing" event before "Normalizing" (the job already starts
  in that state).
- The peak-ceiling test signal is a short faded burst once per second (the plan's burst didn't hit the ceiling).

## Follow-ups

- **GUI startup timeouts:** `bpm_zero_has_no_clicks_and_bad_bpm_text_is_refused` and
  `qa_malicious_server_stem_names_are_refused_in_the_ui` failed in every full GUI run of the last builds and always
  pass alone. Worth a small task to give them more start-up time or run them later.
- Requirement 8 (log line with loudness, true peak and gain) is in the code and was checked by the reviewer, but no
  test asserts it.
- **Disk:** `/` has about 15 GB free; `cargo clean` frees the build output in `target/`.

## How to try it

Laptop: pull, `npm run build:app` in `src/calliope-gui/`, install as before. No settings or `track.json` change.
The calliope-stems server on archserver needs no change; it now gets a 24-bit FLAC more often (16-bit CD rips after
a gain), which it already accepts.

## Manual checks (laptop, real server on archserver)

- [ ] **Quiet song:** Local Audio File, an old quiet CD rip: "Normalizing the audio..." with a bar, then "Normalized
  to -14.0 LUFS (gain +x.x dB)" in the track details. Extract with "keep original" on; in the Player the track's
  "original" plays at about the level of a newer track's.
- [ ] **Loud master from a URL:** the line shows a negative gain; the stems sound normal.
- [ ] **Very dynamic piece** (classical, live): "limited by the -1 dBTP peak ceiling", a level below -14 LUFS.
- [ ] **Directory import:** "Normalizing the audio..." for each file, one result per saved track in the summary.
- [ ] **Cancel:** a 10+ minute file, Cancel while normalizing: the source page comes back, nothing new in the Library,
  no `file-...` folder in `<repository>/import-tmp/`.
- [ ] **Resume:** import a URL, close Calliope at the track details, start again, same URL, Resume: the details come
  back without the Normalizing step and with the same result line.
- [ ] **Timing:** note how long the step takes for a 10-15 minute track (expected well under a minute).

Roadmap: add/tick `gui-import-normalize.md` in `specs/overview.md` once the manual checks pass.
