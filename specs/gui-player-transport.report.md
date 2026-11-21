# Report: gui-player-transport

Status: **DONE**

Branch `spec/gui-player-transport`, base `6cbe015`. Not pushed or merged.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| No tempo, a BPM is set: the section's Click is off, Play has no clicks | `player::tests::the_first_bpm_on_a_track_starts_with_click_off_and_is_silent` (exact PCM) | PASS |
| Autodetect (or section BPM detection) makes sections: every new one off | `qa_p3_autodetect_carry_takes_only_section_1_click_and_count_in`, `tempo_map::tests::with_play_settings_of_carries_section_one` | PASS |
| Split or Add: the new section is off | `qa_p2_typed_change_makes_a_section_with_click_off`, `player::tests::new_sections_are_off_and_only_they_are_written_off`, `qa_pt_a_typed_bpm_on_a_track_with_a_click_on_section_keeps_it_and_the_new_one_is_silent` | PASS |
| An older `player.json` with sections on (no key) and off keeps each, also after saves | `player_state::tests::click_flags_read_and_write_the_same_as_before`, `qa_pt_old_files_keep_on_and_off_through_edits_saves_and_a_reopen` | PASS |
| Playing at 1:23.4, To start: playback goes on from 0:00.0 | `player::tests::jump_to_start_beat_snap_and_range`, `qa_pt_jump_ends_clamps_and_never_leaves_the_song`, `ControlBar.test.ts`; GUI `player_transport` | PASS |
| Rew at 0:02.0 gives 0:00.0; FF 3 s before the end gives the end | `qa_pt_jump_ends_clamps_and_never_leaves_the_song`; GUI `player_transport` | PASS |
| 120 BPM, a click at 0:10.2: playhead at 0:10.0, section selected, keeps playing | `tempo_map::tests::nearest_beat`, `jump_to_start_beat_snap_and_range`, `WaveformView.test.ts`; GUI `player_transport` | PASS |
| No tempo, a click at 0:10.2: playhead at 0:10.2 | `jump_to_start_beat_snap_and_range`, `qa_pt_a_click_never_goes_before_the_set_first_beat_and_no_tempo_clicks_are_exact`; GUI `player_transport` | PASS |
| A click on a marker or label drags or renames; the playhead doesn't move | `WaveformView.test.ts` (no seek on a label or marker, none while locked, none in the Editor) | PASS |
| 1280x800 no scrollbar; 1024x640 no horizontal scroll | GUI `player_transport`, `qa_p3_layout_never_clips_sideways`: overflow 0 / hscroll 0 at 1280x800; hscroll 0 at 1024x640 | PASS |

The tester also checked:
- clamps for huge, negative, NaN and infinite positions;
- a set first beat: To start, Rew and clicks never go before it;
- a click before the first section snaps to the first tempo's beats continued backwards;
- a jump while playing with a section Count In keeps playing from the target.

Suites: `npm test` passes on `de990c0`. In the whole GUI suite (on `7d0c3fa`) every Player transport test passed, but
16 others failed because the disk filled up during the run ("No space left on device") and from the known startup
timeouts. All 16 passed when rerun.

## Task log

| Step | By | Commit |
|---|---|---|
| plan, `docs/architecture.md` | architect | 6f3666f |
| task 1: new sections start with Click off | implementer | 10c5917 |
| tasks 2-3: nearest beat, `player_jump` | implementer | efe0e9b |
| task 4: frontend `toStart`, `seekClick` | implementer | bad8c62 |
| task 5: To start button, layout log fields | local model (exact, no fixes needed) | dd84fa6 |
| task 6: click to seek on the Player waveform | implementer | db4612f |
| task 7: GUI tests, button coordinates, `docs/ui.md` | implementer | 7d0c3fa |
| tester: 6 more acceptance tests, no defects | tester | de990c0 |

## Review

APPROVED, no blockers or majors; no fix round. The reviewer confirmed:
- every place that creates a section starts it with Click off;
- a missing key still reads as on, the file format is unchanged and the previous build reads the new files;
- `player_jump` is registered everywhere, refuses bad positions and clamps to the song and the first beat;
- the Editor's waveform and the Player slider's seek guard are unchanged.

Minor, left as is:
- With a set first beat, To start goes to that beat, not 0:00.0 (decided by the team; recorded in
  `docs/architecture.md`).
- The nearest-beat search looks 6 s either side, so a grid sparser than that (below about 10 BPM) wouldn't snap.
- With no tempo the playhead can land 1 ms off the click (rounding to an audio frame).

## Deviations from the plan

- The plan expected a backend `player jump` log line; the GUI test uses the frontend's log lines instead.
- The GUI test reads the Rew / FF positions from the saved `player.json`, as no log line shows them.
- No GUI test needed changing for the new Click default.

## Follow-ups

- **Disk:** `/` has about 7 GB free and filled up during the GUI run. `target/` takes 45 GB (36 GB of old test
  binaries). `cargo clean` in the repository root frees it, at the cost of one full rebuild.
- The startup timeouts in full GUI runs are still open (see earlier reports).

## How to try it

Laptop: pull, `npm run build:app` in `src/calliope-gui/`, install as before. No settings or file format change. No
server change.

## Manual checks (laptop, real audio device)

- [ ] **New tempo is quiet:** a track with no tempo, type a BPM, Play: no clicks. Press Click: the clicks start on
  the beat.
- [ ] **Old tracks keep their clicks:** a track whose sections clicked before still clicks; a silent one stays
  silent.
- [ ] **Add / split is quiet:** on a clicking section press `+` (or type another BPM mid-section) and play across
  the change: the new section is silent, the old one still clicks.
- [ ] **Autodetect is quiet:** Autodetect BPM in the Editor, then play in the Player: no clicks in any section.
- [ ] **To start:** while playing mid-song, To start: the song goes on from the start with no gap.
- [ ] **Click to seek:** with Click on, click the waveform between two beats while playing: the music goes on from
  the nearest beat and the clicks stay with the music. Click on a marker: it drags and the music doesn't jump.
- [ ] **Ends:** Rew near the start stops at 0:00.0; FF near the end stops at the end.
- [ ] **Previous build** (optional): a track saved by this build shows the same Click settings in the previous
  build.
- [ ] **Layout:** at your usual window size To start, Rew, Play, FF are on one line with no scrollbar.

Roadmap: add/tick `gui-player-transport.md` in `specs/overview.md` once the manual checks pass.
