# Player transport: to start, click to seek, metronome off by default

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Make moving around a song in the Player quicker while practising: a button
back to the start, a click on the waveform that puts the playhead on the
nearest beat, and no metronome clicks unless the owner switches them on.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `gui-player-tempo-sections.md`: the Player's tempo map has sections, each
  with its own metronome and count-in flags (`player.json`, schema 5; there
  is no master metronome any more). Today every section starts with its
  metronome on: the default for a new section, a section made by Autodetect
  or by the section BPM detection (`gui-section-bpm-detect.md`), and a split
  keeping the flag of the section it splits. In `player.json` a section with
  the metronome on has no `metronome` key (a missing key reads as on).
- The Playback group at the bottom of the Player has Rew (-5 s), Play/Pause,
  FF (+5 s), the time field, the position slider and Speed. A seek while
  playing keeps playing.
- The Player's waveform (`WaveformView`, free-move mode): a click on a
  section marker starts dragging it, a click on a section label renames it,
  a click anywhere else selects the section under the mouse. The playhead
  does not move on a click.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. **Metronome off by default:** every section made from now on starts with
   its metronome off, on every track (one section or several): the first
   section of a track without a tempo, sections from Autodetect and from the
   section BPM detection, and sections added or split by the owner. The
   owner switches it on per section as today.
2. **Saved sections keep their setting:** sections already saved in
   `player.json`, including those saved with the metronome on (no
   `metronome` key), keep their saved setting.
3. **To start:** the Playback group gets a "to start" button before Rew,
   in the order To start, Rew, Play/Pause, FF. It moves the playhead to
   0:00.0.
4. **Song range:** To start, Rew, FF and a waveform click never move the
   playhead before 0:00.0 or past the song's end.
5. **Click to seek:** a click on the waveform (not on a section marker or a
   label) moves the playhead to the beat of the tempo grid nearest the click
   and still selects the section under it. With no tempo set, it moves to
   the clicked position. Dragging a marker and renaming a label work as
   today.
6. **While playing:** a jump made by these buttons or a waveform click
   keeps playing from the new position, like a slider seek today.
7. Loops, count-in, speed, the slider, the time field and the keyboard
   shortcuts behave as today.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- Older `player.json` files read as before (requirement 2); files written by
  this build stay readable by the previous build where possible, with any
  schema change noted in `docs/architecture.md`.
- Tests use temporary directories and the test audio backends only: never
  the real audio device, real dialogs or the user's real repository.
- The Player pane keeps fitting at 1280x800 without a scrollbar, and at
  1024x640 without horizontal scroll.
- No `symphonia`.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

A separate Stop button. Changing the Rew/FF step. Changing the metronome of
sections already saved. Click to seek in the Editor.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given a track without a tempo, when a BPM is set, then its section's
  metronome is off and Play sounds no clicks
- [ ] Given Autodetect (or the section BPM detection) makes several
  sections, then every new section's metronome is off
- [ ] Given a section is split or added, then the new section's metronome is
  off
- [ ] Given a `player.json` saved by the previous build with sections whose
  metronome is on (no key) and off, when the track is opened, then each keeps
  its setting, and after any save they still do
- [ ] Given the playhead at 1:23.4, playing, when To start is pressed, then
  playback goes on from 0:00.0
- [ ] Given the playhead at 0:02.0, when Rew is pressed, then it is at
  0:00.0; given 3 s before the end, when FF is pressed, then it is at the end
- [ ] Given a tempo of 120 BPM from 0:00.0, when the waveform is clicked at
  0:10.2, then the playhead goes to 0:10.0 and the section under the click is
  selected; while playing, playback goes on from there
- [ ] Given a track without a tempo, when the waveform is clicked at 0:10.2,
  then the playhead goes to 0:10.2
- [ ] Given a click on a section marker or label, then the marker drags or
  the label renames as today and the playhead does not move
- [ ] Given the Player at 1280x800, then there is no scrollbar; at 1024x640
  no horizontal scroll

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

None. Decided by the owner on 2026-11-21:
1. **Metronome off for every new section,** on every track; saved sections
   keep their setting.
2. **Buttons:** To start, Rew, Play/Pause, FF; Rew/FF stay 5 s; Play/Pause
   stays one toggle (no Stop).
3. **Click snaps to the nearest beat;** with no tempo, the exact position.
4. **Jumps while playing keep playing.**
