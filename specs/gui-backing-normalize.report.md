# Report: gui-backing-normalize

Status: **DONE**

Branch `spec/gui-backing-normalize`, base `903a903`. Not pushed or merged.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| EBU reference signals: loudness and true peak within 0.1 LU / 0.1 dB | `ac1_ebu_reference_signals` (16 and 24 bit); `loudness_tests` (EBU 3341 cases 1-5, true-peak 15-19 at 48 / 44.1 / 22.05 kHz: worst 0.036 LU, 0.054 dB) | PASS |
| A -20 LUFS mix is saved at -14.0, the line says gain +6.0 dB | `ac2_a_minus_20_lufs_mix_is_saved_at_minus_14`; `loudness-format.test.ts` | PASS |
| A mix whose gain would pass -1 dBTP: peak at -1 dBTP, loudness below -14, the line says limited | `ac3_a_mix_whose_gain_would_pass_the_ceiling_is_limited_by_it`; `backing_normalize_tests` (high-crest signal) | PASS |
| A mix that clipped before: no clipped sample, no warning | `ac4_a_mix_that_clipped_before_is_saved_without_a_clipped_sample`; GUI `play_solo_nudge_save_clip` (+12 dB Save: `clipped=0`) | PASS |
| Normalize on an old backing: -14.0, same length and stems, `track.json` records loudness and gain | `ac5_normalize_brings_an_existing_backing_to_minus_14_and_touches_nothing_else`; GUI `editor_normalize` | PASS |
| Already normalized: not rewritten, the line says so | `ac6_an_already_normalized_backing_is_not_rewritten`; GUI `editor_normalize` (bytes and mtime unchanged) | PASS |
| Cancel: the file is byte-identical | `ac7_cancel_during_normalize_leaves_the_file_byte_identical`, `qa_cancel_in_the_measuring_phase_leaves_everything_as_it_was`; GUI `editor_normalize` | PASS |
| No file yet: Normalize disabled | `ac8_normalize_is_refused_without_a_backing_file`; `acceptance_editor.test.ts`; GUI `editor_normalize` | PASS |
| Track open in the Player: it plays the normalized file | `ac9_the_player_plays_the_normalized_file_after_normalize`, `qa_normalize_with_the_player_playing_closes_it_once_and_the_audio_stays_valid`; GUI `editor_normalize_player_reopen` (tone level matches the logged gain within 0.3 dB, loops and tempo unchanged) | PASS |

The tester also covered: other sample rates (8 to 96 kHz) with a mono stem; 24-bit Save within 0.02 LU; tracks
under 400 ms and digital silence ("too quiet or too short", written as mixed); a stale or hand-edited loudness record
(measured again, not trusted); unknown keys in the record; the second variant left alone; the old file in the trash
byte for byte; a file changed between measuring and the swap (refused as a conflict, the other change kept).

Suites on `36f08bf`: `npm test` passes and the whole GUI suite passes (all 9 binaries, first try). One unrelated
calliope-stems log-timing test failed once in `npm test` and passed on every rerun.

## Task log

| Step | By | Commit |
|---|---|---|
| plan | architect | dca99a7 |
| plan: `ebur128` pinned to `=0.1.10` (0.1.11 was two days old) | me | 04a57cc |
| task 1: `ebur128` dependency | local model | 00ef89c |
| task 2: `mixer::sum` | local model | 32773fe |
| tasks 3-5: loudness meter, `track.json` record, safe file replace | implementer | 5f091fa |
| tasks 6-8: Save normalizes, Normalize job, Player release, command | implementer | 7dbcbbf |
| tasks 9-10: result texts, Normalize button | implementer | 6b66921 |
| tasks 11-13: acceptance tests, Editor GUI tests, docs | implementer | 8d289d6 |
| tester: 11 more acceptance tests, no defects | tester | 5a62baf |
| fix round 1 | implementer | 36f08bf |

The local model did tasks 1 and 2 correctly, with no changes needed.

## Review

APPROVED, no blockers or majors; APPROVED again after fix round 1. The reviewer checked the lock order (Player lock,
then repository lock, everywhere: no deadlock), the undo steps of the file swap, and cancel in every phase. Fix round 1:
- the 1024x640 GUI check compares with the overflow measured before, instead of a fixed pixel value;
- the "too quiet" line now reads "Too quiet or too short to normalize" (it also covers tracks under 400 ms);
- an open of the track that Normalize cancels in the Player is retried and gets the new file (now tested).

Left as is: when Normalize finds a backing at the target without a matching record, the record it writes says gain
0 dB, dropping the gain of an earlier write. Harmless; the file isn't touched.

## Deviations from the plan

- **The +12 dB Save is turned down, not limited:** a mix that loud measures far above -14 LUFS, so Save lowers it by
  about 11 dB (outcome "normalized"). The ceiling-limited case needs a quiet but peaky mix; it is tested headlessly.
- **1024x640:** the Editor pane scrolls there, as it did before this feature; the Save row is below the fold. The Mix
  lane doesn't grow.
- **"Save as: ..."** is cut off with "…" at 1280x800, since Normalize takes room beside it; the full name is in its
  tooltip.
- The Normalize job refuses files that aren't 16 or 24 bit, with a message, instead of failing halfway.
- Normalize button is 134 px wide (plan: 120).

## Follow-ups

- Plain backing tracks (imported, no stems) can't be normalized yet; a Library action could do it.
- Autodetect BPM isn't greyed out while a Save or Normalize runs (the backend refuses it).
- Save keeps the whole encoded FLAC in memory before writing (as before): about 100-160 MB for a 15-minute track.

## How to try it

Laptop: pull, `npm run build:app` in `src/calliope-gui/`, install as before. No migration: old backings stay as they
are until you Save again or press Normalize. calliope-stems on archserver is not touched.

## Manual checks (laptop)

- [ ] **Save normalizes:** open a stem track in the Editor, set a mix, Save. The line reads "Saved, normalized to -14.0
  LUFS (gain ...)" or "Saved at ... LUFS (gain ..., limited by the -1 dBTP peak ceiling)". In the Player it sounds like
  the mix, only louder or quieter as a whole (no pumping, no distortion).
- [ ] **A hot mix:** push stems to +12 dB until CLIP lights, Save. No clipped-samples warning (the gain is negative);
  the backing doesn't crackle in the Player.
- [ ] **Normalize an old backing:** on a track saved before this version, press Normalize: progress runs, the button
  reads Cancel, then "Normalized to -14.0 LUFS (gain ...)". Press it again: "Already at the target (-14.0 LUFS); the
  file was not changed".
- [ ] **Cancel:** on a long track, Normalize then Cancel at once: "Normalize cancelled; the file was not changed", and
  the Player still plays the old level.
- [ ] **Player follows:** play the track in the Player, Normalize it in the Editor, go back: it reopens with the same
  loops and tempo and plays at the new level.
- [ ] **Same level across tracks:** normalize three or four backings and switch between them in the Player and on the
  RC-5 after an export: no volume change needed (very dynamic tracks may still be quieter: the ceiling limited them).
- [ ] **Optional cross-check:** `ffmpeg -nostdin -i backings/backing.flac -af ebur128=peak=true -f null -` on a
  normalized file: "I:" is -14.0 ± 0.1 LUFS (or the limited value shown) and the true peak at most -1.0 dBFS.
- [ ] **Layout:** at 1280x800, Save, Normalize and "Save as: ..." are on one line and the Mix lane doesn't grow when a
  result appears.
- [ ] **Trash:** after a Normalize, the repository's `trash/` has the old file under `<stamp>-<id>-backing/`.

Roadmap: add/tick `gui-backing-normalize.md` in `specs/overview.md` once the manual checks pass.
