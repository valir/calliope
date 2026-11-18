# Normalize the audio at import

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Some imported songs are mastered much quieter (or louder) than others. Make
the first step of every import bring the song to the same loudness, so the
stems, the kept original and everything made from them start at a consistent
level.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- Imports (`gui-stem-extraction.md`, `gui-existing-track-import.md`,
  `gui-import-local-batch.md`): the source is a URL (yt-dlp), a local audio
  file, a local video file or a directory of files. Each is converted with
  ffmpeg to FLAC 44.1 kHz stereo, sent to the edge-AI server for stem
  extraction, edited, then saved as stems (and, with the "keep original"
  setting, the full mix as `original.flac`).
- `gui-backing-normalize.md`: the backing Save and the Editor's Normalize
  already bring a backing to -14 LUFS integrated (ITU-R BS.1770), with the
  true peak at most -1 dBTP and no limiter: gain = the smaller of
  "-14 - loudness" and "-1 - true peak"; very quiet or very short audio is
  left as it is. The meter (`src/loudness.rs`) and its rules exist.
- Nothing is normalized at import today: a quiet song gives quiet stems.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. **First step:** right after the source is converted, and before it is sent
   for stem extraction, the full mix is normalized with the backing rules:
   -14 LUFS integrated, true peak at most -1 dBTP, no limiter.
2. **Every import source:** URL, local audio file, local video file and every
   file of a directory import.
3. **What uses it:** the stems are extracted from the normalized mix, and the
   kept `original.flac` is the normalized mix.
4. **Shown:** the import progress shows a "Normalizing" step, and the result
   is shown with the track before saving, e.g. "Normalized to -14.0 LUFS
   (gain +6.0 dB)", "Normalized to -16.2 LUFS (gain +3.1 dB, limited by the
   -1 dBTP peak ceiling)", "Already at -14.0 LUFS; not changed" or "Too quiet
   or too short to normalize; imported as is". A directory import lists it
   per file in its summary.
5. **Never worse:** a song already within 0.1 LU of the target is not
   changed; the normalized mix never has a sample above full scale.
6. **Cancel and failure:** Cancel during this step stops the import as Cancel
   does today; a failure of this step fails the import with its reason and
   leaves nothing behind, like the other steps.
7. **Resume:** an import resumed after a restart does not normalize the mix a
   second time.
8. **Logged:** the measured loudness, true peak and applied gain are written
   to the log.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- Reuse the existing loudness meter and rules; no new loudness library.
- No `symphonia`.
- Tests use temporary directories, the fake edge-AI server and fixture files
  only: never the real calliope-stems server, a real URL download, real
  dialogs or the user's real repository.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

Re-normalizing tracks already in the library (the Editor's Normalize covers
backings). Normalizing each stem on its own. A limiter or compressor. A
setting to choose another target.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given a local file at -24 LUFS with headroom, when it is imported, then
  the mix sent for extraction measures -14.0 ± 0.1 LUFS and the result line
  says "gain +10.0 dB"
- [ ] Given a file whose gain would push the true peak above -1 dBTP, then
  the mix sent has a true peak of at most -1 dBTP, measures below -14 LUFS,
  and the line says it was limited by the peak ceiling
- [ ] Given a file already at -14.0 LUFS, then the mix sent is
  sample-identical to the converted file and the line says "not changed"
- [ ] Given digital silence or a file under 0.4 s, then the import goes on
  with the file as converted and the line says "Too quiet or too short"
- [ ] Given "keep original" on, then `original.flac` measures the same as the
  mix sent for extraction
- [ ] Given a URL, a video and a directory import (fakes), then each file is
  normalized before extraction, and the directory summary shows each file's
  result
- [ ] Given Cancel during "Normalizing", then nothing is saved and the
  import's temporary files are removed
- [ ] Given an import resumed after a restart, then the mix is not
  normalized again (same bytes as before the restart)

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

None. The proposed defaults were confirmed by the owner on 2026-11-18:
1. **Same target as the backings:** -14 LUFS, -1 dBTP ceiling, no limiter;
   very dynamic songs may end a little below -14 LUFS.
2. **Always on,** no switch to turn it off.
3. **The kept original is the normalized mix** (not the file as
   downloaded), so the Player's "original" plays at the same level as the
   rest.
4. **Shown, not stored:** the result is shown during the import and logged,
   not recorded in `track.json` (the backing Save measures its own mix again
   anyway).
