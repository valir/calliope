# Local import: smarter Browse and whole-directory import

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Make importing local files quicker: the Browse dialog opens where the user
is likely to look, and a whole directory of songs can be turned into stem
tracks in one go instead of one import per file.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `gui-stem-extraction.md` built the Import view: "Stem Extraction" opens the
  "Select source" page with URL, Local Audio File and Local Video File. A
  local file is chosen by typing its path or with Browse, then Extract
  prepares it, shows the track edit pane with the file's metadata, extracts
  the stems on the edge-AI server, drops the empty stems and saves the track.
- `gui-existing-track-import.md` added typed paths next to Browse.
- Today Browse always opens the system file dialog in the home directory,
  whatever was used before or typed in the box.
- Only one import runs at a time.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. Browse (Local Audio File and Local Video File) opens the file dialog in
   the first of these that applies:
   1. the path typed in the path box, when the box is not empty and the path
      is valid: an existing file opens its folder, an existing directory
      opens that directory. Relative and `~` paths are understood the same
      way as for Extract;
   2. otherwise the folder of the last file imported from the local disk;
   3. otherwise the home directory.
   An invalid typed path is not an error for Browse: it falls back silently.
2. "The last location" is the folder of the last local file (or directory,
   requirement 3) whose import was accepted, whether it was typed or picked.
   It is saved in the app settings and survives a restart. It is
   forgotten silently when that folder no longer exists.
3. The "Select source" page gets a fourth option, "Local Directory", with a
   path box, a Browse button that opens the system folder dialog (starting
   as in requirement 1) and an Extract button.
4. Extract on a directory looks for audio files directly inside it (the same
   formats as Local Audio File, by extension, case-insensitive; not in
   subdirectories) and imports them one after the other, in file-name order,
   each exactly as a single Local Audio File import would (preparation,
   metadata, stem extraction, empty stems dropped, track saved).
5. When the directory has no audio files, nothing starts and the page shows
   "Directory <dir> has no audio files" under the path box. A path that is
   not an existing directory shows an error there as well.
6. While a directory import runs, the page shows which file is being
   imported and how far the batch is ("File 3 of 8: song.mp3") together with
   that file's progress, and lists the files already done.
7. A file that fails (unreadable, the server fails, ...) is reported and the
   batch goes on with the next file.
8. Cancel stops the current file and skips the rest; the tracks already saved
   are kept.
9. At the end, a summary lists the tracks saved (with their dropped stems)
   and the files that failed with the reason, with "Show in Library" and
   "Import another" as after a single import.
10. Each started, finished, failed and cancelled file of a batch is written to
    the log, as single imports are.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- Tests never open the real file or folder dialog (use the existing fake
  picker) and use the fake stems server, never the real calliope-stems on
  0.0.0.0:8765 or archserver.
- No symphonia. Files and directories are only read, never changed.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

Subdirectories. Video files inside a directory. Several imports at the same
time. Drag and drop. Choosing several files in the file dialog. Skipping
songs that are already in the library.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given the path box holds the path of an existing audio file, when the
  user presses Browse, then the dialog opens in that file's folder
- [ ] Given the path box holds an existing directory, when the user presses
  Browse, then the dialog opens in that directory
- [ ] Given the path box is empty or holds a path that doesn't exist, and a
  file from /music/rock was imported before, when the user presses Browse,
  then the dialog opens in /music/rock, also after an app restart
- [ ] Given nothing was imported yet and the path box is empty, when the user
  presses Browse, then the dialog opens in the home directory
- [ ] Given "Local Directory" is selected, when the user presses Browse, then
  the folder dialog opens and the chosen directory fills the path box
- [ ] Given a directory with b.mp3, a.flac, cover.jpg and a subdirectory with
  c.mp3, when the user presses Extract, then a.flac and then b.mp3 are
  imported, each as its own stem track, and cover.jpg and c.mp3 are ignored
- [ ] Given a directory with no audio files, when the user presses Extract,
  then "Directory <dir> has no audio files" is shown and no import starts
- [ ] Given a directory import of 3 files where the second one fails, then
  the first and third tracks are saved and the summary lists the second one
  with its error
- [ ] Given a directory import running on file 2 of 3, when the user presses
  Cancel, then the first track is kept, the second and third are not
  imported, and the page returns to an idle state
- [ ] Given a directory import is running, then the page shows "File N of M:
  <name>" and the current file's progress

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

1. The track edit pane in a directory import: skip it and use each file's
   own metadata (file name as the title when it has none), so the batch runs
   unattended and the tracks are edited later in the Library? Or show the
   edit pane for every file before its extraction? (Proposed: skip it.)
2. One shared "last location" for audio files, video files and directories,
   or one each? (Proposed: one shared.)
3. Does the file dialog also preselect the typed file, when the system
   dialog supports it, or only open its folder? (Proposed: only the folder.)
