# Player speed and speed training

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Let the musician slow a backing track down (or speed it up) by a percentage,
with the pitch kept, and practise a loop that speeds up by itself on each
repeat until it reaches the target speed.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `gui-play-backing-track.md` requirement 14 described speed as BPM "-"/"+"
  steps of 5 with "Reset". That was never built: the Player only has hidden
  placeholders for those buttons and a stored `speed` factor (0.5..1.5) that
  is always 1.0. This spec replaces that requirement with a percentage
  control.
- `gui-player-tempo-sections.md` built per-section tempos, per-section Click
  and Count In, and the Sections / Loops / Playback layout.
- Other practice apps work the same way (checked 2026-10-18): Transcribe!,
  Amazing Slow Downer, Anytune and Guitar Pro use a percentage (or ×)
  control, typically 25..200 % in 5 % steps; apps that know the tempo show
  the resulting BPM. Guitar Pro and Soundslice offer speed training (start %,
  end %, step, repeats per step).

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. Speed control: a percentage from 25 % to 200 %, default 100 %, changed in
   steps of 5 % with "-" and "+", typed directly (whole numbers), and reset to
   100 % with one button. It applies to the whole track.
2. The pitch is kept at any speed; only the tempo changes.
3. A speed change while playing takes effect at once, without stopping or a
   gap, and the playhead stays where it is.
4. The position, the time display, the waveform, section starts and loop
   boundaries stay in song time: a loop from 0:10 to 0:20 is still shown as
   0:10..0:20 at 50 %; it just takes twice as long to play.
5. The metronome and the count-in follow the speed: at 50 % a 120 BPM section
   clicks at 60 BPM and stays on the beats of the music.
6. The Playback pane shows the current BPM read-only: the BPM of the section
   under the playhead multiplied by the speed (e.g. "60 BPM" for 120 BPM at
   50 %). It updates when the playhead enters another section or the speed
   changes. With no tempo map it shows no BPM, and the speed still works.
7. The speed is saved per track in player.json and restored when the track is
   opened again.
8. Speed training, per loop: the user can switch it on for a complete loop
   and set a start %, an end %, a step % and how many repeats to play at each
   speed. Start and end are 25..200 % and may go up or down.
9. With speed training on, Play of that loop starts at the start % and,
   after the given number of repeats, moves one step towards the end %; once
   the end % is reached it keeps repeating at the end %. Each new speed
   applies from the loop start, without a gap.
10. While training, the Loops pane shows the training progress (the current
    speed and the repeat count, e.g. "70 % · 2/4"), and the speed control and
    the BPM readout show the current training speed.
11. Stopping the loop or leaving it ends the training run, and the speed goes
    back to the track speed of requirement 1. The next Play of the loop starts
    the training again from the start %.
12. The training settings of each loop are saved in player.json with the
    loop.
13. Count-in during training follows the existing loop rule (only on the
    first Play, or on every repeat when the loop's Count-In is on), at the
    current training speed.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- The time-stretching code must have a permissive licence (MIT, Apache-2.0,
  BSD or similar): no GPL or LGPL in the shipped app (see `docs/licences.md`).
- It runs on the laptop in real time, on the existing playback path; no
  symphonia. Tests never open the real audio device.
- player.json files written by earlier versions keep working (their
  `speed` becomes the percentage).

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

Changing the pitch (transposing) without changing the speed. Exporting a
slowed-down file. A target-BPM input (the BPM is a readout only). Different
speeds per section. Changes to the Editor view.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given a track at 100 %, when the user presses "-" three times, then the
  speed shows 85 % and 10 s of song time take about 11.8 s to play
- [ ] Given a 440 Hz tone in the track, when it plays at 50 % and at 150 %,
  then the tone stays at 440 Hz (within a few Hz)
- [ ] Given playback at 100 %, when the user sets 70 %, then playback goes on
  without a pause and the position keeps moving forward from the same place
- [ ] Given a 120 BPM section at 50 %, then the Playback pane shows 60 BPM
  and the metronome clicks every 1 s, on the music's beats
- [ ] Given a track without a tempo map, then no BPM is shown and the speed
  control still slows playback down
- [ ] Given the user types 20 or 250, then the value is refused and the speed
  is unchanged
- [ ] Given a speed of 80 %, when the Player is closed and reopened, then the
  speed is 80 %
- [ ] Given a loop with training 60 % → 80 %, step 10 %, 2 repeats, when the
  user plays it, then it plays twice at 60 %, twice at 70 %, then keeps
  repeating at 80 %, and the Loops pane shows the speed and repeat count
- [ ] Given a training run in progress, when the user stops the loop, then
  the speed returns to the track speed, and the next Play starts again at
  60 %
- [ ] Given a loop with training settings, when the Player is closed and
  reopened, then the settings are restored
- [ ] Given a player.json from the previous version, when the track is
  opened, then it loads at 100 % with its loops and sections unchanged

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

- Where the speed control and the training settings sit in the current
  layout (Playback pane next to the BPM readout; training inside the Loops
  pane or in a small popover).
- Does the track speed of requirement 1 also scale during training (e.g.
  training at 60..80 % of an 80 % track), or does training use absolute
  percentages? (Proposed: absolute.)
