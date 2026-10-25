# First beat: place the song's first beat where the music starts

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Music rarely starts at 0:00: there is silence, noise or a count from the
recording first. Let the first section start where the music really starts,
found automatically and then fine-tuned by hand, so the Player's count-in
and the exported count-in lead exactly into the first beat.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- Tempo sections (`gui-player-tempo-sections.md`, `stems-beat-autodetect.md`)
  are a list of segments. The first segment's `start_ms` is bar 1, beat 1,
  and its tempo extends backwards to 0:00. Section start bars are drawn on the
  waveform in the Editor and in the Player, and can be dragged (in 1/64-beat
  steps, at most 4 beats from where they were detected or added).
- Every count-in is six ticks over 4 beats, the 5th silent, then the music's
  beat (fix 2026-10-25): on Play and loop wraps, before a section with Count
  In on, and in front of every export (`gui-export.md`, requirement 12).
- The export already places its count-in from the first section's start, so
  a correct first beat makes the exported count-in correct too.
- The owner's tests (2026-10-25): count-ins into middle sections sound right;
  the first section's count-in is off because the first beat isn't where the
  music starts.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. **Find the start:** Calliope finds where the music actually starts: the
   first point where the signal rises clearly above the leading silence or
   noise, judged relative to the track's loudest part. It places the first
   section's start bar there. A track whose music starts at 0:00 keeps its
   first beat at 0:00.
   1. This runs automatically once, the first time a track with a tempo but
      no found first beat is opened in the Player.
   2. A "Find start" button in the Player runs it again and moves the first
      beat to the found position.
2. **Adjust by hand**, in the Editor and in the Player: the first section's
   start bar can be dragged and moved with the keys like the other section
   start bars, anywhere from 0:00 up to the next section's start. The readout
   shows its offset from the found position.
3. **Before the first beat is not played:** the audio left of the first
   section's start is shown dimmed on the waveform and is left out of
   playback: Play from the beginning starts at the first beat (after the
   count-in when the first section has Count In on), and the playhead can't
   be placed before it.
4. **Export:** the export leaves out the audio before the first beat: the
   file is the 5-beat count-in lead followed directly by the audio from the
   first beat on (length = 5 beats + the track's length after the first
   beat). This replaces "the original + the count-in" of `gui-export.md`
   (owner answer 1 there) and its "intro kept as is".
5. Moving the first beat moves the first section's beat grid with it (the
   first section's start is its bar 1, beat 1). It doesn't move the other
   sections' starts, so the last beat before the second section gets longer
   or shorter; the second section's start can then be adjusted by hand as
   usual.
6. The first beat is saved with the track's tempo, and the tracks saved
   before this feature play and export as they do today until the first beat
   is found or moved.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- Tests use temporary repositories only, never the user's real repository,
  real devices or the real system dialogs.
- No `symphonia`.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

Trimming the end of the track. Detecting the beat or the tempo itself (beat
autodetection already exists). Editing the audio files: the part before the
first beat stays in the files.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given a backing track with 1.2 s of low noise before the music, when
  the start is found, then the first section starts within 10 ms of 1.2 s
- [ ] Given that track, when the first section's bar is dragged 1/64 beat
  later, then it moves and the readout shows the offset from the found
  position
- [ ] Given a first beat at 1.2 s, when Play is pressed at the beginning with
  the first section's Count In on, then the six ticks and the silent beat are
  heard and the music starts at 1.2 s exactly on the 6th beat position
- [ ] Given a first beat at 1.2 s, then the playhead can't be moved before
  1.2 s, and the region before it is dimmed
- [ ] Given a first beat at 1.2 s, when the track is exported, then the file
  is the 5-beat lead followed by the audio from 1.2 s on, the 5th lead beat
  silent and ending exactly at the first beat, and the noise before 1.2 s is
  not in the file
- [ ] Given a track with a tempo and no found first beat, when it is opened in
  the Player for the first time, then the first beat is placed at the found
  start; when the bar is moved by hand and "Find start" is pressed, then it
  goes back to the found start
- [ ] Given a track with several sections, when the first beat is moved,
  then the other sections' starts are unchanged and the first section's
  beats follow the new first beat
- [ ] Given a first beat dragged left past 0:00 or right past the next
  section's start, then it stops at 0:00 / just before the next section
- [ ] Given a track saved before this feature, then it plays and exports as
  before

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

Answered by the owner (2026-10-25): in the Editor and the Player; found
automatically once, plus a "Find start" button in the Player; the export
leaves out the audio before the first beat; the first beat moves anywhere
from 0:00 to the next section's start; the detection level is relative to
the track's loudest part; the first section's grid moves with the first beat
and the other sections' starts stay (adjusted by hand as usual).

Still open: an existing track's first section may already start where the
owner put it (typing a BPM while paused on the first downbeat sets the grid
there). The automatic search is skipped on a track whose first beat is not set
at 0.0, but the "find start" button would still be able to change it.

