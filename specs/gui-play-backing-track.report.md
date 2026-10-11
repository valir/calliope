# Report: play a backing track (increment A) and the switch to MPL-2.0

**Status: DONE (increment A).** All 28 accepted criteria in scope for increment A pass. The reviewer approved,
after two fix rounds. Increment B (speed change keeping the pitch) waits for your choice of time-stretch library.
Detect (BPM detection) moves to its own later spec.

- Branch: `spec/gui-play-backing-track` (base `aa9a84b`), not pushed or merged
- Plan: `specs/gui-play-backing-track.plan.md` (revision 2). Log: `specs/gui-play-backing-track.log.md`
- **Nothing to deploy on archserver.** Only the desktop app changed.

## What you get
- **Player view** (Alt+5):
  - the Library tree on the left, which Ctrl+L folds to a narrow "Library" strip;
  - on the right, the "Play:" dropdown, the tablature placeholder, the waveform and the control bar.
- **Dropdown:**
  - It lists the backings, the plain backing audio, "Original mix" and the stems.
  - On a track's first visit it picks the first backing, else the first stem. The original mix is never picked
    automatically.
  - Changing it stops playback and keeps the position.
- **Playback:**
  - Play/Pause, Rew/FF (5 s), the position slider, and the time field (`M:SS.T`, editable).
  - Space toggles playback. Leaving the view pauses.
- **BPM:**
  - Typing a BPM sets the tempo from the current position to the end, or to the next change. 0 removes the tempo in
    effect there.
  - "V" is ticked when the track has several tempos.
  - `-`, `+`, Reset and Detect are visible but disabled ("Comes in a later update").
- **Metronome:**
  - Our own click, mixed into the same audio stream, so it can't drift: 2 kHz on beat 1, 1.5 kHz on the others.
  - Count In plays 3 audible clicks, and the music starts on the silent 4th beat.
  - Both need a BPM.
- **Loops:**
  - `+` starts a loop at the current position with no end. Loop Play, Loop On and Count-In are disabled until you
    press `<`.
  - `>` and `<` snap to the nearest 1/64 in BPM mode, and store exact times at BPM 0.
  - Prev/Next (or ←/→) jump at once. `b`/`e` mark the start and end, and `L` toggles Loop On.
  - Loop On repeats without a click at the joint. The loop's own Count-In plays before every pass.
- **Waveform:** it stretches with the window and shows a playhead. Loops are drawn in amber (your choice): two marks
  and a bar, or only the start mark for a loop with no end.
- **Saved settings:** `player.json` next to `track.json`. It holds the source, tempo, metronome, loops, the current
  loop and the playback position. It is written 0.5 s after a change and every 10 s while playing, with no Save
  button.
  - An unreadable file, or one from a newer version, is shown as a note in the tablature box. A newer file is never
    overwritten.
  - A failed save is shown in red.
- **Licence:**
  - Calliope is now under **MPL-2.0**. `LICENSE` is byte-identical to mozilla.org's text (checked with `cmp`), and the
    crates, npm package, README, `docs/licences.md` and the notices all match.
  - The "no symphonia" rule still holds, with a note that you can reconsider it.
- **Settings:** the Audio output and MIDI placeholders now point to a later `gui-audio-midi-settings` feature.

## Acceptance criteria
The criteria are in plan section 7. The tester's tests are in `tests/acceptance_player_backend.rs`,
`src/ui/acceptance_player.test.ts`, `tests/gui_player_acceptance_e2e.rs` and `tests/acceptance_player_licence.rs`.

| # | Criterion | Result |
|---|---|---|
| 1 | Ctrl+L folds and restores the tree, and the selection is kept | PASS |
| 2 | The track selected in the Library is loaded in the Player | PASS |
| 3 | No track, or no audio: inactive pane, controls disabled | PASS |
| 4 | Dropdown, tablature area, waveform and four sections shown | PASS |
| 5, 6 | Sources listed; backing selected first, else the first stem; original never picked automatically | PASS |
| 7 | Source choice saved within 1 s and restored after reselect or restart | PASS |
| 8 | Changing the source stops playback (keeps the position) | PASS |
| 9 | Tablature placeholder | PASS |
| 10-12 | Play/Pause, time field, Rew/FF, all clamped to the track | PASS |
| 13 | BPM 0: timestamps, metronome, count-in and `-`/`+`/Reset disabled | PASS |
| 14-16 | Typed BPM, later tempo change snapped to 1/64, V marker, 0 removes the tempo | PASS |
| 17 | Speed (increment B) | not built |
| 18 | Detect | moved to the later spec |
| 19 | Metronome on every beat, accent on beat 1, follows tempo changes | PASS (exact samples) |
| 20 | Count-in: 3 clicks, starts on the 4th beat | PASS (exact samples) |
| 21 | Count-in before a loop's first pass; the loop's Count-In before every pass | PASS |
| 22-26 | Add/remove, Prev/Next, `>`/`<` snapping, loop Play/Stop/Loop On, read-only in-loop slider | PASS |
| 27 | Waveform fills the width, amber loop marks | PASS (vitest and screenshots) |
| 28 | Every setting on disk within 1 s and restored; problems with `player.json` shown | PASS (after fix round 1) |
| 29 | Leaving the Player pauses | PASS |
| 30-33 | Loop with no end: disabled controls, `<` completes it or is refused, `>` past the end clears the end, survives restart | PASS |
| 34 | Paused position restored after restart | PASS |
| - | Licence MPL-2.0 everywhere | PASS |

Suites:
- **Headless** (`env -u DISPLAY npm test`): green. vitest 502 tests; Rust workspace 3402 passed, 0 failed; clippy
  clean.
- **GUI** (`DISPLAY=:1 npm run test:gui`): 51 tests in 8 binaries, green. This includes the 7 Player e2e tests and the
  tester's 5. They check the captured audio for the clicks and the tone.

## Task log

| Task | Done by |
|---|---|
| 1 Fixtures | implementer |
| 2 Reserve `player.json` | **local model** (exact edit, correct) |
| 3-9 Tempo map, state, sources, metronome, engine (two tasks), IPC | implementer |
| 10 Bar/BPM formatting | implementer (the local model wrote no files) |
| 11 Waveform geometry | **local model**: correct code but in the wrong folder; moved and checked by the orchestrator |
| 12-17 Layout, frontend state, control bar, waveform, keys, GUI e2e + visual review + docs | implementer |
| 18 Licence to MPL-2.0 | implementer |
| tester | 28 PASS, 4 defects |
| reviewer | CHANGES REQUIRED (1 major) |
| fix round 1 | implementer; reviewer APPROVED, tester confirmed |
| fix round 2 | implementer (overlapping messages) |

**Local model:** 1 of 3 routine tasks was clean. It handles an exact edit to an existing file. On new files it either
wrote nothing or wrote them in the wrong place, and both times it claimed the tests passed.

**Fix round 1:**
- **Major:** `player.json` problems were never shown, so changes could be lost silently.
- **Loops:** loops were unreachable when the file had no valid current loop.
- **Extreme values:** overflow on extreme numbers in the file or in a seek.
- **Empty audio:** an empty audio file crashed the audio thread.
- **Keys:** held keys repeated play/pause.
- **Loop times:** typed loop times weren't range-checked.
- **Loop On:** looked pressed while disabled.

**Fix round 2:** the note and the save error overlapped when both showed.

**Visual review (task 17):**
- The controls were clipped at 1280x800; they now wrap.
- Pressed buttons were unreadable in the dark theme; they are now amber.
- A flaky position slider was fixed.

## Deviations from the plan
- **Loop index in logs:** the log's loop index is 0-based (`loop=0`), and `docs/architecture.md` was corrected.
- **1024x640:** with the tree folded, the controls scroll vertically. The plan's "nothing scrolls" can't be met at that
  size.
- **Status messages:** `player.json` messages appear inside the tablature box, so the layout doesn't shift when there
  are none.
- **Manual check 5:** corrected in the plan. `b` marks the start of an existing loop, and doesn't create one.
- **Calling ffmpeg:** `calliope-lib` gained `spawn_raw_stdout`. The existing helper only carries text lines, not PCM.

## Remaining (minor, optional)
- **Both engines sounding:** if Play is pressed in the Editor and in the Player at almost the same moment, both could
  sound. Each stops the other first, but not under one shared lock.
- **Editor solo button:** it uses the same amber/black classes and probably has the same dark-theme look as the
  Player's pressed buttons had. Not touched, because it is outside this feature.
- **Typed loop time past the end:** shown as a general action error, not as a specific loop message.
- **Position after save/restore:** a position that isn't on a whole audio frame loses about 1 ms on each save and
  restore. It settles after 2-3 cycles.
- **Metronome at BPM 0:** the metronome flags stay stored at BPM 0 and come back when a BPM is typed again.

## Next
- **Increment B (speed):** pick the library and I'll build tasks 19-22.
  - **signalsmith-stretch (MIT)** is recommended. It needs `clang`.
  - **Bungee (MPL-2.0)** is the alternative. It needs `cmake`.
- **Detect:** write `gui-play-backing-track-detect.md` when you want it. The plan is "Beat This!" on archserver.

## How to try it
```sh
cd src/calliope-gui && npm run build:app && ../../target/release/calliope-gui
```
To try it with test data, copy `src/calliope-gui/tests/fixtures/library-player` to a new folder and choose it in
Settings. The "With Tempo" track has two tempos and three loops, one of them with no end.

## Manual checks (laptop, real audio)
- [ ] Alt+5, a real stem track with a backing: the dropdown shows "Backing", the waveform shows loud and quiet parts,
  Play sounds on the default output, and the time and slider move smoothly
- [ ] While playing, switch the dropdown to Guitar: playback stops, and Play continues on the guitar from the same place
- [ ] Pause on the first downbeat, type the song's BPM, Enter. Metronome On + Play: the ticks are on the beat, accent on
  beat 1
- [ ] Count In on, Play: three ticks, then the music starts on the 4th beat, in time
- [ ] Press `+` at a chorus start: only a start mark appears, and loop Play is disabled. Press `<` (or `e`) at its end:
  the amber bar appears. Loop On + loop Play: it repeats with no click or gap. Turn the loop's Count-In on: a
  count-in plays before each pass
- [ ] Add more loops and use Prev/Next or the arrows: the jumps are instant, and the marks match what you hear
- [ ] Pause mid-song, close and restart: the track, source, BPM, loops and position are back, and `player.json` is
  readable
- [ ] An old plain backing track with MP3 audio plays (needs ffmpeg)
- [ ] Ctrl+L folds and restores the tree
- [ ] Switch to the Library while playing: playback pauses. Back in the Player, Play resumes in the same place
- [ ] From 1-2 m in a dim room: the BPM, time, section number and loop marks are readable. Space, b, e, arrows and L
  work
- [ ] `LICENSE` and the About/notices say MPL-2.0

Once verified, tick `gui-play-backing-track.md` in the roadmap of `specs/overview.md`. It stays partly open until
increment B is built.
