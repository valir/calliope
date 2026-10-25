# Report: gui-first-beat

Status: **DONE**

Branch `spec/gui-first-beat`, base `f1c29e9` (the unmerged `fix/player-count-in` commits are included).
Not pushed or merged.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| 1.2 s of noise before the music: the first section starts within 10 ms of 1.2 s | `fb_the_start_is_found_within_10_ms_of_1200`; `first_beat.rs` unit tests; GUI `player_first_beat` | PASS |
| Drag 1/64 beat later: it moves, the readout shows the offset from the found position | `fb_drag_one_64th_later_moves_by_one_fine_step_from_the_found_position`; `WaveformView.test.ts`; GUI `player_first_beat`, `editor_first_beat` | PASS |
| First beat at 1.2 s, Play with Count In: six ticks, a silent beat, music at 1.2 s on the 6th beat | `fb_play_from_the_beginning_counts_in_then_starts_the_music_at_1200_exactly`, `fb_without_count_in_play_starts_at_the_first_beat_at_once`; `player_tests.rs`; GUI `player_first_beat` (captured WAV) | PASS |
| The playhead can't go before 1.2 s; the region before it is dimmed | `fb_the_playhead_cannot_be_placed_before_the_first_beat`, `fb_moving_the_first_beat_past_the_playhead_pulls_it_along_and_playing_continues`, `fb_loops_that_start_before_the_first_beat_never_play_it`; `WaveformView.test.ts` (dim); GUI `player_first_beat` | PASS |
| Export: the 5-beat lead then the audio from 1.2 s, 5th beat silent, no noise | `ac_first_beat_rc5_is_the_lead_then_the_audio_from_the_first_beat` (bit-exact), `ac_first_beat_mp3_and_ogg_have_the_same_length`, `ac_first_beat_fractional_position_and_odd_tempo_are_sample_exact`; `export_audio_tests.rs` | PASS |
| Opened for the first time: placed at the found start; moved by hand then "Find start": back to it | `fb_automatic_placement_sets_the_first_beat_once_and_find_start_returns_to_it`, `fb_the_first_beat_is_saved_with_the_tempo_and_restored`; `player_ipc_tests.rs`; GUI `player_first_beat` | PASS |
| Several sections: the others stay, the first section's beats follow | `fb_other_sections_are_unchanged_and_the_first_sections_grid_follows`, `ac_first_beat_with_several_sections_uses_the_first_sections_tempo_and_start_only`; `tempo_map.rs` | PASS |
| Dragged past 0:00 or the next section: stops at 0:00 / one beat before it | `fb_limits_stop_at_zero_and_one_beat_before_the_next_section`; `waveform-geometry.test.ts`; GUI `editor_first_beat` | PASS |
| A track saved before this feature plays and exports as before | `fb_a_track_saved_before_the_feature_plays_from_zero_and_keeps_its_tempo`, `fb_a_legacy_track_with_a_count_in_still_counts_in_from_where_it_is`, `ac_first_beat_track_saved_before_the_feature_exports_everything`; the existing schema-6 export tests | PASS |

Suites on `6bb53aa`: `npm test` passes and the whole GUI suite passes (all 9 binaries).

## Task log

| Step | By | Commit |
|---|---|---|
| plan | architect | 23af0ed |
| tasks 1-3: tempo map first beat, `player.json` schema 7, start detector | implementer | 57b4e9a |
| task 4: noisy-intro fixture | local model (header tidied by me) | af920d9 |
| task 5: player floor, placement, moving bar 1 | implementer | 43e94c4 |
| tasks 6-7: automatic run, Find start, Editor move | implementer | b7a39dd |
| task 8: export without the lead-in | implementer | f9f6537 |
| tasks 9-12: waveform helpers and view, Find start button, Export text | implementer | f370215 |
| tasks 13-15: GUI tests, docs | implementer | c0133b5 |
| tester: 21 acceptance tests, no defects | tester | 557ab26 |
| fix round 1 | implementer | 6bb53aa |

The local model did task 4 successfully. Its task 9 result was discarded (wrong `playStartMs` formula, tests
written to match it) and the implementer redid it.

## Review

APPROVED, no blockers or majors, and APPROVED again after fix round 1. Fix round 1 handled the minors:
- the automatic run no longer fails an open when there is no room for the first beat (it logs
  `reason=no-room`);
- a superseded open stops its analysis decode;
- the drag preview can't go negative when the limits cross;
- the Editor GUI test that waits for a loop save got a longer deadline (it flaked once under full load).

Left as is: if an audio file is replaced by one shorter than the saved first beat, the Player opens at the end
and Play does nothing until the first beat is moved or found again.

## Deviations from the plan

- **Find start button** is smaller (`text-sm`) than the other section buttons, so the Player still fits at
  1280x800 with Loops beside Sections. A bigger button would need something else in that row to give.
- **Editor focus** on the waveform now starts on the first bar; Down selects the next change. One existing GUI
  test was updated for it.
- **Editor log fix:** first-bar moves logged `applied=0`; fixed in tasks 13-15.
- The dimmed lead-in is barely visible on the test fixture (very quiet noise); the dim is checked by the
  unit and GUI tests, not by eye.

## Follow-ups

- In the full parallel GUI run, the first test of a binary sometimes times out while the app starts (no log
  line after GTK start). Each of these passed on rerun; it's load, not this feature. Worth a longer startup
  wait in `tests/common` if it keeps happening.
- `player.json` is now schema 7: once this build saves a track, older builds open it read-only.

## How to try it

Laptop: pull, `npm run build:app`, install as before.

## Manual checks (laptop)

- [ ] Open a real song with silence, noise or a spoken count before the music, with a tempo whose first beat
  is at 0:00, in the Player. The first bar jumps to where the music starts; the part before it is dimmed.
- [ ] Turn on the first section's Count In, press Home, then Play: six ticks, a silent beat, and the music's
  first beat right on the next count.
- [ ] Drag the first bar a little, or select section 1 on the waveform and use Left/Right, Shift+Left/Right.
  The readout shows the offset from the found start; the count-in follows.
- [ ] Press "Find start": the bar goes back to the found position.
- [ ] On a track with several sections, move the first beat: the later bars stay, and the Click still sits on
  every section's beats.
- [ ] Export that track to the RC-5 and to MP3: the file starts with the count-in leading straight into the
  music, without what came before. The Export pane says how much is left out.
- [ ] Open an older track whose first beat you placed yourself (not at 0:00): nothing moves, Play from the start
  still plays the intro, the export sounds as before. Then press "Find start" once.
- [ ] In the Editor, the same track shows the dimmed lead-in and the first bar moves with the keys; Editor
  playback still starts at 0:00.

Roadmap: tick `gui-first-beat.md` in `specs/overview.md` once the manual checks pass.
