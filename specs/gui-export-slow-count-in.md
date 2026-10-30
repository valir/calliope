# Slow count-in and lead silence in exports

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

On fast tracks (heavy metal) the exported count-in goes by too quickly to come
in on time. Let the owner export a count-in at half the track's tempo, and put
silence before every count-in so there is time to get into position after
pressing play on the RC-5.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `gui-export.md` requirement 12 and `fix/count-in-4-beats`: every export
  (RC-5, MP3, OGG) starts with a count-in at the first section's BPM: 4 beats,
  6 ticks at beats 0, 1, 2, 2.5, 3, 3.5 ("one, two, one-two-three-four"), and
  the track's first beat right after, at beat 4. Nothing comes before the first
  tick.
- `gui-first-beat.md`: with a set first beat, the audio before it is dropped,
  so the file is the count-in plus the audio from the first beat.
- The Export pane shows the count-in line, e.g. "Count-in: 4 beats at 180 BPM
  before the first beat (adds 1.3 s)".
- The Player's own Play count-in is not affected by this feature.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. **Slow count-in option:** the Export pane has a "Slow count-in (half tempo)"
   switch. When it is on, the count-in is played at half the first section's
   BPM: the same 6 ticks with the same pattern, each spacing twice as long
   (ticks at beats 0, 2, 4, 5, 6, 7 of the track's tempo), and the first beat
   right after, at beat 8.
2. **Normal count-in:** when the switch is off, the count-in is exactly as
   today.
3. **Lead silence:** every exported file starts with silence as long as its
   count-in (4 beats normally, 8 beats with the slow count-in, at the track's
   tempo), then the count-in, then the track. The audio of the track itself is
   unchanged.
4. **Per track:** the switch is remembered per track (kept across app
   restarts) and shown again whenever that track is selected; a track never
   set uses the normal count-in.
5. **All targets:** the same count-in and lead silence go into RC-5, MP3 and
   OGG exports.
6. **The count-in line** says which count-in will be written and how much the
   export adds in total, e.g. "Count-in: 4 beats at 90 BPM (half of 180),
   after 2.7 s of silence (adds 5.3 s)".
7. The Export pane still fits at 1280x800 and 1024x640 without overflow.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- Tests use temporary directories only: never a real RC-5, a real drive or the
  user's real repository, and never the real system dialogs.
- No `symphonia`.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

The Player's Play / loop count-in. Other count-in patterns or speeds than half
tempo. A separately adjustable silence length. Re-exporting existing files.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given a 120 BPM track with the switch off, when it is exported, then the
  file starts with 4 beats (2.0 s) of silence, the 6 ticks follow at beats
  0, 1, 2, 2.5, 3, 3.5 after it, and the track's first beat is at 4.0 s
- [ ] Given the same track with the switch on, when it is exported, then the
  file starts with 8 beats (4.0 s) of silence, the ticks follow at beats
  0, 2, 4, 5, 6, 7 after it, and the first beat is at 8.0 s
- [ ] Given either setting, then every sample from the first beat on is
  identical to the export without the lead silence (shifted by its length)
- [ ] Given a set first beat, then the audio before it is still dropped
- [ ] Given the switch turned on for track A and off for track B, when the app
  is restarted and A is selected, then the switch is on; for B it is off
- [ ] Given the switch on, when the track is exported to MP3 or OGG, then the
  file has the same slow count-in and silence
- [ ] Given the switch toggled, then the count-in line changes at once to the
  matching text and length
- [ ] Given the Export view at 1280x800 and 1024x640, then nothing overflows

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

None. The proposed defaults were confirmed by the owner on 2026-10-30:
1. **Silence for every export,** with the normal count-in too (1 bar of silence
   normally, 2 bars with the slow one), not only with the slow count-in.
2. **Silence length = the count-in's length** (4 or 8 beats at the track's
   tempo), so with the slow count-in the RC-5 plays 2 bars of silence, then 2
   bars of count-in.
3. **Remembered per track** in the track's own data, so a batch re-export keeps
   each track's choice; off for every existing track.
4. **All three targets** (RC-5, MP3, OGG) get the same count-in and silence.
