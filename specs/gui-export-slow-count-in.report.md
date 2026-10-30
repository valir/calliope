# Report: gui-export-slow-count-in

Status: **DONE**

Branch `spec/gui-export-slow-count-in`, base `882b763`. Not pushed or merged.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| 120 BPM, switch off: 2.0 s silence, ticks at beats 0, 1, 2, 2.5, 3, 3.5 after it, first beat at 4.0 s | `ac_slow_off_120_bpm_silence_ticks_first_beat`; unit `plan_normal_and_slow_at_120_bpm` | PASS |
| Switch on: 4.0 s silence, ticks at beats 0, 2, 4, 5, 6, 7 after it, first beat at 8.0 s | `ac_slow_on_120_bpm_silence_ticks_first_beat` | PASS |
| Every sample from the first beat on is identical (shifted by the lead) | `ac_slow_audio_from_the_first_beat_is_unchanged` (bit-exact, both settings) | PASS |
| A set first beat: the audio before it is still dropped | `ac_slow_first_beat_set_still_drops_the_audio_before_it` | PASS |
| Switch on for A, off for B: kept across a restart; a track never set is normal | `ac_slow_per_track_survives_a_restart`; GUI `export_slow_count_in` (restart step) | PASS |
| MP3 and OGG get the same slow count-in and silence | `ac_slow_mp3_and_ogg_have_the_same_count_in_and_silence` | PASS |
| The count-in line changes at once to the matching text and length | `ac_slow_info_texts_values`; `ExportView.test.ts`, `export-format.test.ts`; GUI `export_slow_count_in` | PASS |
| Nothing overflows at 1280x800 and 1024x640 | GUI `export_slow_count_in`, `export_rc5_quick`: overflow 0 at 1280x800; hscroll 0 at both sizes | PASS (see deviations: 1024x640 scrolls vertically) |

The tester also checked:
- odd beat lengths (117.3 and 143.7 BPM) and 40 and 300 BPM, switch on and off;
- a 48 kHz source resampled to 44.1 kHz;
- a hand-edited or junk `exports.slow_count_in` (read as normal; switching on repairs it);
- toggling during a running export (the running file stays as started, the next one follows);
- toggling while the overwrite question is open;
- toggling with an open Library edit (no conflict).

Suites on `ab1596b`: `npm test` passes. In the whole GUI suite every test passed except one Player test that hit the
known app-startup timeout and passed when rerun alone (before the fix round: two such tests elsewhere, same cause).

## Task log

| Step | By | Commit |
|---|---|---|
| plan | architect | addeb97 |
| tasks 1-3: per-track storage, lead silence and slow count-in, export command | implementer | 6385405 |
| task 4: register the command | local model | 93412b2 |
| tasks 5-7: acceptance tests, window state and texts, the switch | implementer | 4974ed5 |
| task 8: README | local model | 0b95893 |
| task 9: GUI tests, layout, `docs/ui.md` | implementer | 7d6fe26 |
| tester: 6 more acceptance tests, no functional defects | tester | 5f663c2 |
| fix round 1: overflow 0 at 1280x800, no export while the switch saves | implementer | ab1596b |

The local model did tasks 4 and 8 correctly, with no changes needed.

## Review

CHANGES REQUIRED first (one major: 3 px overflow at 1280x800 against requirement 7; two minors), then APPROVED after
fix round 1. The reviewer confirmed:
- the frame math (one rounded beat, ticks and first beat consistent, the first-beat skip kept, `run` unchanged);
- the key only lives under `exports`: the track's revision and `modified` don't change, older builds keep it;
- the flag is read when the export starts, so the file matches the pane when the overwrite question is answered;
- the Player's count-in is unchanged.

Left as is (minor): Change... next to the RC-5 folder stays enabled for the instant the switch saves; it starts no export.

## Deviations from the plan

- **The overflow check was wrong before this build.** It measured an inner element that never scrolls, so it always
  read 0, including the "overflow 0" I reported for the quick RC-5 export. It now measures the part that scrolls.
- **Tighter Export pane** to fit at 1280x800: smaller gaps, the three export buttons from 3.25 to 2.75 rem high, less
  padding under the "Export..." group. The buttons moved (MP3 434, OGG 542 in the GUI tests).
- **1024x640:** the Export pane scrolls vertically by 230 to 244 px there (the OGG button is below the fold); there is
  no horizontal scroll. This is accepted as the existing limit at the minimum window size, as in the Player and Editor,
  and written in `docs/ui.md`.
- The switch follows the saved state (function binding), so a failed save leaves it as it was.

## Follow-ups

- If the 1024x640 scrolling bothers you, the fixed-height "Last:" and result rows are the place to save room.
- **Startup timeouts:** the full parallel GUI run still hits app-startup timeouts in a few tests now and then.

## How to try it

Laptop: pull, `npm run build:app` in `src/calliope-gui/`, install as before. One new command inside the app;
`track.json` gains the optional `exports.slow_count_in` (only when on). No server change.

Note: every export is now longer than before, also with the switch off (the silence is new).

## Manual checks (laptop, real RC-5)

- [ ] **Normal count-in with silence:** a fast track (e.g. 180 BPM), switch off. The line reads "Count-in: 4 beats at
  180 BPM, after 1.3 s of silence (adds 2.7 s)". Export to an RC-5 memory and play it: about 1.3 s of silence, then
  "one, two, one-two-three-four" at 180, and the track on the beat after.
- [ ] **Slow count-in:** switch on. The line at once reads "Count-in: 4 beats at 90 BPM (half of 180), after 2.7 s of
  silence (adds 5.3 s)". Export to the same memory (Yes to replace) and play: 2 bars of silence, the count at half
  speed, then the track at full speed. Check you can come in on time.
- [ ] **Per track:** switch on for track A, off for B, export both, restart Calliope: A shows on, B off. Re-export A:
  still slow.
- [ ] **First beat:** a track with a set first beat (Find start): the export starts right at the first beat after the
  count-in, with nothing of the intro.
- [ ] **MP3 / OGG:** switch on, export both and play them: the same silence and slow count-in.
- [ ] **RC-5 loop:** with the RC-5 looping, the silence and count-in play at every repeat. Tell me if the silence makes
  the restart awkward.
- [ ] **Look:** at your usual size the switch sits under the count-in line with no scrollbar; at about 1024x640 the
  pane scrolls but nothing overlaps.

Roadmap: add/tick `gui-export-slow-count-in.md` in `specs/overview.md` once the manual checks pass.
