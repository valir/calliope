# Export: a track's backing to the BOSS RC-5, MP3 or OGG

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Take a finished backing track out of Calliope: copy it to the BOSS RC-5
looper for playing without the laptop, or save it as an MP3 or OGG file to
use elsewhere.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- The Library tree (`gui-tracks-repository.md`, ordered by
  `gui-library-album-order.md`) lists bands, albums and tracks; the Editor and
  Player views reuse it read-only.
- A stem track's backing track is made in the Editor
  (`gui-backing-track-editor.md`): Save renders the mix of the chosen stems
  to `backings/backing.flac` and lists it in `track.json` `backings` (the
  first variant is the default). A stem track without a saved mix has no
  backing track.
- A plain backing track (imported by `gui-existing-track-import.md`) has its
  audio in the file named by `track.json` `audio`.
- ffmpeg is already a runtime prerequisite (local imports, non-FLAC
  playback).
- The BOSS RC-5 shows up as a USB drive with a `ROLAND` folder at its root.
  Each of its 99 memories reads one WAV file with a fixed name:
  `ROLAND/WAVE/0NN_1/0NN_1.WAV` (memory 1 is `001_1/001_1.WAV`, memory 99 is
  `099_1/099_1.WAV`).

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. A new **Export** view in the navigation shows the Library tree (read-only,
   same order, search, keyboard navigation and Collapse/Expand all) on the
   left and, for the selected track, its band, album and title and an
   "Export..." group on the right with three buttons: "to BOSS RC-5",
   "MP3 file" and "OGG file".
2. The buttons are enabled only for a track that has a backing track: a stem
   track with a saved mix (its default backing variant), or a plain backing
   track (its `audio` file). Otherwise they are disabled and the pane says
   why (e.g. "No mix saved yet: save one in the Editor"). Nothing is enabled
   when a band or an album is selected.
3. Only the backing track is exported: never the stems, the kept original
   or the tablatures.
4. **to BOSS RC-5:**
   1. The user picks the base directory (the root of the mounted RC-5) with
      the system folder dialog and enters the memory number, 1 to 99.
   2. The file is written as `<base>/ROLAND/WAVE/0NN_1/0NN_1.WAV` (NN = the
      number on two digits, e.g. `007_1/007_1.WAV`), creating `WAVE` and
      `0NN_1` when they are missing.
   3. The file is always a WAV, 32-bit float, stereo, 44.1 kHz, with no
      metadata (no tags or extra chunks): a backing track at another rate is
      resampled, and a mono one is turned into stereo (the same signal on
      both channels).
   4. The export is refused with a message, and nothing is written, when:
      the number is out of range; the base is not an existing directory; or
      the base has no `ROLAND` folder ("No ROLAND folder in <base>: is the
      RC-5 mounted?"), since that means the drive's root was not chosen.
   5. Other files in the memory's folder are left alone.
5. **MP3 file / OGG file:** the user picks the destination directory with
   the system folder dialog, and the backing track is encoded there with the
   matching codec (MP3, or Ogg Vorbis):
   1. named `<Band> - <Title>.mp3` / `.ogg`, with characters that are invalid
      in file names replaced;
   2. MP3 VBR quality 2 (about 190 kbps), Ogg Vorbis quality 6 (about
      192 kbps), at the backing track's own sample rate;
   3. tagged with the title, the band as artist, the album, the track
      number (its position, when it has one) and the year, when known.
6. An export never changes the track, its files or the repository.
7. An existing file is never overwritten without a confirmation that names
   it. Cancelling the confirmation writes nothing.
8. While an export runs, the pane shows its progress and a Cancel button. A
   cancelled or failed export leaves no partial file behind, and a failure
   shows its reason. Success shows the full path of the written file.
9. Exports are written to the log (started, done with the path, failed,
   cancelled).
10. The RC-5 base directory and the MP3/OGG destination directory are
    remembered separately across restarts, and the folder dialog opens
    there. The memory-number box suggests the last number used plus one
    (1 the first time, and 99 stays 99).
11. Export comes after Player in the navigation (Alt+6); Settings becomes
    Alt+7.
12. When preparing the exported track, we always prepend the count-in to the
    exported file, so user can effectively use the backing track on the looper
    or in the other destination. The count-in is being organized on 4
    preceding beats of the first beat of the track. 6 ticks spaced as [1/4, 1/4, 1/8
    repeated 4 times].

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- Tests use temporary directories only: never a real RC-5, a real drive or
  the user's real repository, and never the real system dialogs.
- No `symphonia`.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

Exporting several tracks or an album at once. Exporting stems, a single
stem or the original mix. Choosing among several backing variants. Other
formats (FLAC, WAV outside the RC-5) and other loopers. Reading back,
listing or deleting what is on the RC-5. Setting RC-5 memory settings
(tempo, loop mode, names). Exporting from the Library, Editor or Player
views.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given a stem track with a saved mix, when it is selected in the Export
  view, then the three export buttons are enabled
- [ ] Given a stem track without a saved mix, then the three buttons are
  disabled and the pane says that no mix is saved yet
- [ ] Given a band or an album is selected, then the three buttons are
  disabled
- [ ] Given a plain backing track, then the three buttons are enabled
- [ ] Given base directory `B` with a `ROLAND` folder and memory 7, when the
  RC-5 export finishes, then `B/ROLAND/WAVE/007_1/007_1.WAV` exists: 32-bit
  float, stereo, 44.1 kHz, no metadata, the same length and sound as the
  backing track
- [ ] Given base directory `B` without a `ROLAND` folder, then the export is
  refused with the "is the RC-5 mounted?" message and nothing is written
- [ ] Given `B/ROLAND/WAVE/007_1/007_1.WAV` already exists, then the
  overwrite confirmation names it, and after confirming it holds the new
  track
- [ ] Given a mono backing track, when it is exported to the RC-5, then the
  WAV is stereo with the same signal on both channels
- [ ] Given memory 0, 100 or "abc", then the export is refused with a
  message and nothing is written
- [ ] Given a backing track at 48 kHz, when it is exported to the RC-5,
  then the WAV is 44.1 kHz with the same duration
- [ ] Given destination `D`, when the MP3 export finishes, then `D` holds
  `<Band> - <Title>.mp3`, tagged with the title, band and album, that
  decodes to the backing track's duration
- [ ] Given destination `D`, when the OGG export finishes, then `D` holds an
  Ogg Vorbis file that decodes to the backing track's duration
- [ ] Given the target file already exists, when the user cancels the
  overwrite confirmation, then the existing file is unchanged
- [ ] Given an export in progress, when the user presses Cancel, then no
  file is left in the destination
- [ ] Given a destination that cannot be written, then the export fails
  with its reason and the track is unchanged
- [ ] Given any export, then the track's folder and `track.json` are
  unchanged afterwards

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

Answered by the owner (2026-10-23): plain backing tracks are exportable; the
RC-5 file is always 32-bit float, 44.1 kHz, stereo (resampled, mono made
stereo) and named `0NN_1.WAV` inside `0NN_1/`; a base without a `ROLAND`
folder is refused, since the RC-5's mounted root was missed; the proposals
for quality, tags, file names, remembered folders and navigation are kept
(requirements 5, 10 and 11).
