# Detect one section's BPM

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Let the owner re-detect the BPM of a single section from a short piece of audio
chosen with the playhead, for the sections where the whole-track detection got the
tempo wrong.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `stems-beat-autodetect.md` detects the tempo sections of the whole track from
  the Editor (Autodetect BPM): the stems are mixed, uploaded to the edge-AI
  server (`POST /v1/beats`), and the returned beats are turned into sections.
- Some sections still need their BPM fixed by hand. Where a part of the song
  carries less rhythm (e.g. the drums pause), the beats around it are poor and
  they pull that section's BPM off. A short stretch of steady playing inside the
  section gives a better value.
- The Player's Sections pane acts on the section under the playhead; its BPM
  can be typed, doubled or halved (`gui-player-tempo-sections.md`). It also
  holds the Start row with Find start (`gui-first-beat-tuning.md`).

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. **Detect BPM button** in the Player's Sections pane, acting on the section
   under the playhead. Disabled when the track has no tempo map, while a
   detection runs, and when the track is read-only.
2. **The audio analysed** starts at the playhead and lasts 3 bars at the
   section's current BPM (3 × beats per bar × 60 / BPM seconds). If the section
   ends sooner, it stops at the section's end. Nothing before the playhead or
   after the section's end is used.
3. **The detection** sends that piece of the source selected in the Player's
   source dropdown to the edge-AI server's beat tracker, as the Editor's
   Autodetect BPM does, and derives one BPM from the returned beats, rounded
   like the other BPMs (0.1). A piece shorter than 1 bar is refused with a
   message, without calling the server.
4. **Applying it** replaces the BPM of the whole section, exactly as typing that
   BPM on the section would: the section's start, the other sections and their
   BPMs are unchanged; the change is saved with the track.
5. **Feedback:** while it runs, the pane shows that it is detecting and offers
   Cancel; Cancel leaves the section unchanged. When it ends, the status says
   the old and the new BPM (e.g. "BPM 92.0 → 93.4, detected from 0:41.2 –
   0:47.1"). The detected BPM is applied as it comes; when it is about twice or
   half the old one, the status also points to ×2 / ÷2. If the server can't be
   reached, finds no steady beat, or the piece is too short to measure, a
   message says so and nothing changes.
6. Playback can continue during the detection; the new BPM applies live when it
   arrives, as a typed BPM does. If the section is edited meanwhile (moved,
   removed, its BPM typed) or another track is opened, the result is dropped.
7. The button sits on the Start row, after Find start. The Player still fits
   at 1280x800 with Loops beside Sections.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- Uses the existing beats API of calliope-stems; no server change unless the
  architect finds one is needed for short pieces.
- Tests use temporary repositories only, the stub beat tracker and a
  calliope-stems on 127.0.0.1, never the real server, real devices or the real
  system dialogs.
- No `symphonia`.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

Moving the section's start or adding changes from the detection. Detecting in
the Editor. Choosing the length of the piece (always 3 bars). Undo.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given a section at 100 BPM (4/4) and the playhead 10 s into it, when
  Detect BPM is pressed, then the audio sent is the 7.2 s from the playhead
- [ ] Given the playhead 3 s before the section's end, when Detect BPM is
  pressed, then the audio sent is those 3 s only
- [ ] Given the playhead less than 1 bar before the section's end, when Detect
  BPM is pressed, then a message says the piece is too short and the server is
  not called
- [ ] Given the "audio" source selected in the dropdown, when Detect BPM is
  pressed, then the piece sent comes from that source, not the backing track
- [ ] Given the tracker returns beats at 93.4 BPM for that piece, then the
  section's BPM becomes 93.4, its start and the other sections are unchanged,
  and `player.json` holds the new value
- [ ] Given the detection succeeded, then the status shows the old and the new
  BPM and the time range used; given a detected BPM about twice the old one,
  the status also points to ×2 / ÷2
- [ ] Given the server is down, or returns no steady beat, then a message says
  so and the section keeps its BPM
- [ ] Given a running detection, when Cancel is pressed, then the section keeps
  its BPM and the button is enabled again
- [ ] Given a running detection, when the section is removed or another track
  is opened, then the result is not applied
- [ ] Given playback with the Click on, when the new BPM arrives, then the
  clicks follow the new BPM without stopping playback
- [ ] Given the Player at 1280x800, then the layout has no overflow and Loops
  stays beside Sections

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

Answered by the owner (2026-10-27):
1. **Audio:** the source currently selected in the Player's source dropdown
   (the one being played).
2. **Too short:** a piece under 1 bar (e.g. the playhead right before the
   section's end) is refused with a message, without calling the server.
3. **Half / double:** the detected BPM is applied as it comes; if it is about
   twice or half the old one, the status points to ×2 / ÷2 instead of
   correcting it silently.
4. **Place:** the button goes on the Start row, after Find start.
