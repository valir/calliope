# Export info: remember a track's last export to each destination

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Remember where each track was last exported (which RC-5 memory, which MP3 and
OGG file), show it in the Library, and use it to aim a repeated export at the
same place, so re-exporting a reworked backing track replaces the old copy
instead of filling another memory.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `gui-export.md` added the Export view (to BOSS RC-5, MP3 file, OGG file),
  the overwrite confirmation that names the existing file, and the remembered
  RC-5 folder, MP3/OGG folder and next memory number (last used + 1).
- Its requirement 6 says an export never changes the track or the
  repository. This feature changes that rule: a successful export now records
  its info with the track. The audio, the stems, the tablatures and every other
  field still stay unchanged.
- The Library's track pane shows the track's fields, some of them read-only
  (`gui-tracks-repository.md`; Position from `gui-library-album-order.md`).

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. A successful export records its info in the track's `track.json`, one
   entry per export kind, each replacing that kind's earlier entry:
   1. **RC-5:** the memory number and the date and time of the export (not
      the base folder: there is one RC-5);
   2. **MP3** and **OGG:** the full path of the written file and the date and
      time of the export.
2. A cancelled, refused or failed export records nothing and leaves the
   earlier entry as it was.
3. **One track per RC-5 memory:** when a track is exported to an RC-5 memory
   that another track's entry names, that other track's RC-5 entry is
   cleared, since the memory no longer holds it. Its other entries and fields
   stay unchanged.
4. The Library's track pane shows the recorded info as plain read-only
   rows, one per kind that has an entry, e.g. "RC-5: memory 7
   (2026-10-24 21:30)", "MP3: /home/.../Band - Title.mp3 (2026-10-24 21:32)",
   "OGG: ...". A kind without an entry has no row, so a track never exported
   shows none. Edit mode doesn't make the rows editable.
5. **Re-exporting to the RC-5:** for a track with an RC-5 entry, the memory
   box in the Export view starts at the recorded memory instead of "last used
   + 1". Exporting then writes to the same memory and, since the file exists,
   brings up the existing overwrite confirmation. The user can still type
   another number.
6. **Re-exporting to MP3/OGG:** for a track with an entry of that kind, the
   folder dialog opens in the recorded file's folder (or the remembered MP3/OGG
   folder when that folder no longer exists). The file name stays
   `<Band> - <Title>`, so the same file gets the overwrite confirmation.
7. The Export view shows the recorded info for the selected track too, next
   to each button (e.g. "Last: memory 7, 2026-10-24").
8. Recording the info doesn't disturb an edit in progress in the Library, and
   an export of a track whose metadata can't be written still exports the
   file, then reports that the info wasn't saved.
9. The record, and a cleared entry of another track, are written to the
   log (`export recorded`, `export cleared`).

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- Tests use temporary directories and temporary repositories only, never a
  real RC-5, a real drive, the user's real repository or the real system
  dialogs.
- Tracks written by older Calliope builds still load, and older builds still
  load tracks that have export info.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

A history of every export (only the last one of each kind is kept). Checking
whether the recorded file or RC-5 memory still holds this track. Editing or
clearing the info by hand in the Library. Showing the info in the Editor or
the Player. Searching or sorting the Library by it.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given a track never exported, when it is exported to RC-5 memory 7,
  then its metadata holds an RC-5 entry with memory 7 and the export time,
  and the Library shows "RC-5: memory 7 (...)"
- [ ] Given a track with an RC-5 entry for memory 7, when it is selected in
  the Export view, then the memory box shows 7
- [ ] Given that track, when "to BOSS RC-5" is pressed with the same base,
  then the overwrite confirmation names `.../007_1/007_1.WAV`, and after
  confirming the entry's time is updated and the memory is still 7
- [ ] Given a track with an RC-5 entry for memory 7, when it is exported to
  memory 12, then the entry says memory 12
- [ ] Given track A with an RC-5 entry for memory 7, when track B is
  exported to memory 7, then B's entry says memory 7, A has no RC-5 entry
  (and no RC-5 row in the Library), and A's MP3/OGG entries and other fields
  are unchanged
- [ ] Given a track exported as MP3 to `D`, then its metadata and the Library
  show the full path `D/<Band> - <Title>.mp3`; the same for OGG, and the two
  entries are kept apart
- [ ] Given a track with an MP3 entry in `D`, when "MP3 file" is pressed, then
  the folder dialog opens in `D`
- [ ] Given an export that is cancelled, refused or fails, then the track's
  metadata is unchanged
- [ ] Given a track with export info, when its other fields are edited and
  saved in the Library, then the export info is kept, and it can't be edited
- [ ] Given any export, then apart from the export info the track's metadata
  and files are unchanged
- [ ] Given a track never exported, then the Library shows no export rows

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

Answered by the owner (2026-10-24): a track exported to an RC-5 memory clears
that memory from the track that held it before; the info is stored in
`track.json`; the RC-5 entry keeps only the memory number; the Library shows
plain rows, only for the kinds that have an entry.
