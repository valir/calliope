# Player tempo sections editing

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Let the musician correct the tempo sections of a track in the Player, where
they hear the metronome against the song, without going back to the Editor.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `gui-play-backing-track.md` built the Player's tempo map (typed BPMs, -/+,
  the "V" marker) and `stems-beat-autodetect.md` added detected sections, shown
  read-only in the Player waveform; only the Editor can move a detected change.
- Detection is good but not perfect: on real songs (e.g. "Deja-Vu") the intro
  can come out fragmented or at a wrong tempo, so the user needs to fix the
  sections by ear while playing.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. the Player waveform shows the tempo sections as the Editor does (amber bar
   at each change followed by its BPM) and they become editable there
2. move a change: drag a change bar (or select it and use the arrow keys)
   left or right in 1/64 steps of the left section's BPM; detected changes
   keep the existing limit of 4 beats from their detected position
3. set a section's BPM: click the BPM label of a section and type the new
   value; it applies to that section only, up to the next change
4. add a change: insert a new change at a point of the waveform (e.g. the
   playhead), snapped to the 1/64 grid of the section it splits; its BPM is
   typed by the user
5. remove a change: its section joins the previous section and takes its BPM
6. fold/halve: one action on a section doubles or halves its BPM, for half or
   double tempo mistakes left by the detection
7. every edit takes effect at once for the metronome, the count-in and the
   loops, and is saved in player.json like the other Player settings

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

Time signatures other than 4/4 (they come with the tablatures). Undo history
beyond what already exists. Changes to the Editor view need confirmation first.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given a track with detected sections, when the user drags a change bar
  in the Player, then it moves in 1/64 steps of the left section's BPM and
  stops at 4 beats from its detected position
- [ ] Given a section, when the user types a new BPM on its label, then only
  that section changes and the metronome follows it
- [ ] Given a section, when the user adds a change inside it, then the section
  is split at the nearest 1/64 step and the new section has the typed BPM
- [ ] Given a change, when the user removes it, then its section joins the
  previous one
- [ ] Given a section at 93 BPM, when the user doubles it, then it shows 186
  and the metronome ticks at 186 there
- [ ] Given any of these edits while playing, then playback continues and the
  metronome follows the new map from the next beat
- [ ] Given any of these edits, when the Player is closed and reopened, then
  the edited sections are restored from player.json

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

- How the add/remove/double/halve actions are reached (context menu on the
  waveform, buttons in the BPM section, keyboard shortcuts).
- Does an edited detected change keep its "detected" anchor (and so the 4-beat
  limit), or become a typed change free to move anywhere?
