# Export to the BOSS RC-500

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Besides the BOSS RC-5, the owner has a BOSS RC-500, a looper with two tracks
per memory. Let the Export view write a backing track to an RC-500 memory, on
track 1 or track 2, with its own button so it is never mixed up with the RC-5.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `gui-export.md`: "to BOSS RC-5" opens the folder dialog at the remembered
  RC-5 folder (`rc5_dir`), which must hold a `ROLAND` folder; memory N (1..99)
  is written as `<root>/ROLAND/WAVE/0NN_1/0NN_1.WAV` (IEEE float 32-bit, stereo,
  44.1 kHz, written by Calliope); an existing file is only replaced after a
  confirmation that names it. The Memory box suggests the last memory plus one.
- `gui-export-info.md`: the export is recorded per kind in `track.json`
  (`exports.rc5`: memory and time); the Library shows an "RC-5" row and the
  Export view a "Last: memory 7, 2026-10-24" line; one track per memory (an
  RC-5 export clears the record of any other track naming that memory).
- `gui-export-rc5-quick.md`: after a successful RC-5 export, the next ones in
  the same visit of the Export view skip the folder dialog ("To: <folder>" and
  Change...).
- `gui-export-slow-count-in.md`: every export has the lead silence and the
  count-in (slow or normal, per track).
- The RC-500 is mounted the same way as the RC-5 (a USB drive with a `ROLAND`
  folder). On the RC-500, track 2 of memory N has its own folder and file,
  named with `_2` instead of `_1` (checked on the owner's pedal: `007_2/007_2.WAV`).
- The Export pane has no room left at 1280x800 (overflow 0 with every line
  shown), so a second device block needs a new layout.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. **A separate button:** the "Export..." group gets a "to BOSS RC-500" button,
   separate from "to BOSS RC-5", with its own Memory box (1..99) and a
   "Track" choice of two radio buttons, "Track 1" and "Track 2".
2. **Where it is written:** memory N, track 1 is written as
   `<root>/ROLAND/WAVE/0NN_1/0NN_1.WAV`; track 2 as
   `<root>/ROLAND/WAVE/0NN_2/0NN_2.WAV`. The WAV format is the RC-5's. Other
   files and folders are left alone.
3. **The folder:** the dialog opens at the remembered RC-500 folder, kept
   separately from the RC-5's; it must hold a `ROLAND` folder, as for the RC-5.
4. **Overwrite:** an existing track file is only replaced after the same
   confirmation, naming it; "No" writes nothing. Writing track 2 never touches
   track 1's file, and the other way round.
5. **Quick exports:** the RC-500 gets the same quick-export behaviour as the
   RC-5 (its own "To: <folder>" line and Change...), independent of the RC-5's.
6. **Remembered:** the export is recorded per track as an RC-500 export
   (memory, track and time). The Library shows an "RC-500" row
   ("memory 7, track 2 (2026-11-07 18:30)") and the Export view a "Last:"
   line under the RC-500 button. One track per memory and track: an RC-500
   export to memory 7, track 2 clears the RC-500 record of any other track
   naming memory 7, track 2 (not track 1, and not any RC-5 record).
7. **Suggestions:** the RC-500 Memory box and Track choice follow the selected
   track (its recorded RC-500 memory and track), else the last RC-500 memory
   used plus one (99 stays 99) and Track 1.
8. **Count-in:** the same lead silence and count-in as every export (the slow
   switch applies).
9. RC-5, MP3 and OGG exports are unchanged.
10. **Layout:** at 1280x800 the whole Export pane is visible without a
    scrollbar, with every line shown; at 1024x640 it may scroll vertically but
    never horizontally.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- Tests use temporary directories only: never a real RC-5 or RC-500, a real
  drive or the user's real repository, and never the real system dialogs (the
  scripted picker).
- No `symphonia`.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

Exporting both tracks at once. Writing the RC-500's memory settings (tempo,
loop mode, names). Telling an RC-5 drive from an RC-500 drive automatically.
Other BOSS loopers.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given memory 7 and Track 1, when "to BOSS RC-500" is used on a folder
  `B` with `ROLAND`, then `B/ROLAND/WAVE/007_1/007_1.WAV` is written in the
  RC-5's WAV format
- [ ] Given memory 7 and Track 2, then `B/ROLAND/WAVE/007_2/007_2.WAV` is
  written and an existing `007_1/007_1.WAV` is byte-identical afterwards
- [ ] Given `007_2.WAV` exists, when Track 2 is exported to memory 7, then the
  confirmation names it, and "No" writes nothing
- [ ] Given a folder without `ROLAND`, then the RC-500 export is refused with
  the RC-5's message
- [ ] Given an RC-500 export succeeded, when the next track is exported to the
  RC-500 in the same visit, then no folder dialog opens; an RC-5 export still
  asks
- [ ] Given RC-500 and RC-5 folders were picked, when the app restarts, then
  each dialog opens at its own folder
- [ ] Given track A exported to memory 7, track 2, when track B is exported to
  memory 7, track 2, then A has no RC-500 record any more; a track on memory
  7, track 1 and an RC-5 record on memory 7 keep theirs
- [ ] Given a track exported to memory 7, track 2, when it is selected, then
  the Library shows "RC-500 memory 7, track 2 (...)", and the Export view
  shows memory 7 and Track 2 selected
- [ ] Given the slow switch on, then the RC-500 file has the slow count-in
- [ ] Given the Export view at 1280x800 with every line shown, then there is
  no scrollbar; at 1024x640 there is no horizontal scroll

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

None. Decided by the owner on 2026-11-07:
1. **Where track 2 lives:** `ROLAND/WAVE/0NN_2/0NN_2.WAV` (checked on the
   pedal).
2. **Same WAV format and memory range as the RC-5:** 32-bit float, stereo,
   44.1 kHz; memories 1..99.
3. **Separate remembered folder and last memory** for the RC-500, and its own
   quick-export "To:" line, independent of the RC-5's.
4. **Track choice remembered** with the track's RC-500 record, else Track 1.
5. **One track per memory and track:** exporting to a slot clears only other
   tracks' records of that same slot.
6. **Layout:** the architect may re-arrange the Export pane (e.g. the RC-5
   and RC-500 blocks side by side) so it fits at 1280x800.
