# Import track from local file system

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Perform stem extractions out of an local file music track or out of the local
video file.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. in the import view, when "local audio file" is is selected, then show a
   text box allowing path entry and a browse button next to it to browse for
   music files on the local system using the system file dialog
2. in the import view, when "local video file" is is selected, then show a
   text box allowing path entry and a browse button next to it to browse for
   video files on the local system using the system file dialog
3. Extract the audio track from the video tracks and then treat it the same as
   an audio track import.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given audio file import was selected, when a valid path is given, then
  the corresponding stems should be imported in the library
- [ ] Given video file import was selected, when a valid path is given, then
  the stems corresponding to the audio tracks should be imported in the
  library

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->
