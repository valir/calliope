# <Feature name>

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Import an existing music track and turn it into a stem track and place it in
the repository for later editing.

The function should be made available in the "Import" tab. This is the first
import function we'll add to that tab. Later we'll add the "import existing
backing track" feature, but for now we stick to stem extraction.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. Import music track from YouTube link
2. Import music track from local file in well known formats: mp3, flac, ogg
3. Import sound track out of a video file browsed from the current system
4. Use the edge-ai infrastructure to perform the extraction
5. The resulting stems are introduced into the track repository with the track
   type set to "stem"
6. The repository metadata format will be refactored to accomodate this kind
   of tracks
7. Metadata should be extracted from the input file or stream, then presented
   to the user in an edit step before stem extraction
8. Empty stemps should not be kept in the repository, so some tracks will only
   have, for exemple, bass, drums, guitar and vocals, but not piano or other

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given application started, when "Import" tab is selected, then the
  "Import" tab shows a button labeled "Stem Extraction"
- [ ] Given "Stem Extraction" buton clicked, then Import tab screen switches
  to stem extraction mode and display the "select source" page
- [ ] Given the "select source" page visible, when "URL" option is activated,
  then show the "enter url" text box
- [ ] Given the user typed text in "enter url", when "Extract" button is
  clicked, then URL format is checked
- [ ] Given the URL entered is malformed, when "Extract" button is clicked,
  then an accent color text is shown under the box saying "Entered URL is
  invalid"
- [ ] Given the URL entered is valid, when "Extract" button is clicked, then
  the "Download in progress" progress bar is displayed
- [ ] Given the extraction started, when downloading started, then the
  "Download in progress" is updated
- [ ] Given the extraction started, when downloading errors, then display the
  error in the label under the URL text box saying "Error <HTTP Error Code>
  when attempting download"
- [ ] Given the download started, when download is complete, then place the
  downloaded track in a temporary file inside the repository
- [ ] Given the extraction started, when an existing temporary file is found
  in the repository temporary space then prompt: "Incomplete download
  file from the same URL found" and the buttons "Resume Download" and "Start
  Over"
- [ ] Given the download finished, given the track has metadata, then extract
  metadata and initialize a metadata for the track
- [ ] Given the download finished, display the track edit pane in edit mode so
  the user could enter/correct data
- [ ] Given the track edit pane is displayed, when existing metadata was
  extracted, then fill in the edit pane fields for the user to be able to edit
  them
- [ ] Given the track edit pane is displayed, when the "Extract" button is
  clicked, then start the stem extraction starts
- [ ] Given the stem extraction started, when processing finished, then remove
  the temporary file
- [ ] Given the stem extraction  started, when edge-ai confirms processing
  start, then display a waiting wheel labeled "Working..."
- [ ] Given the stem extraction is ongoing, when operation is complete, then
  save the stems together under a track directory associated with the
  metadata collected at the previous edit step
- [ ] Given the repository has stem tracks, when the Library tab is active,
  then it identifies the stem tracks with a distinctive icon in front of their
  name
- [ ] Given the "select source" page visible, when "Local Audio File" option is activated,
  then show the "Browse" button
- [ ] Given the "select source" page visible, when "Local Video File" option is activated,
  then show the "Browse" button
- [ ] Given a video or audio file was selected, when the file has metadata,
  then extract the metadata into the repository metadata format
- [ ] Given a video file was selected, when the file has audio track, then extract
  the audio track into a temporary file
- [ ] Given a video file was selected, when the file has not an audio track,
  then display and error message "Selected file <file> has no audio track"
- [ ] Given an audio file is available, when audio file has metadata, then
  extract the metadata into the repository format
- [ ] Given an audio file is available, then display the track edit pane with
  the initial extracted metadata is displayed
- [ ] Given the stem extraction is ongoing, when operation is complete, then
  check if any stem is all empty (all zeroes) and drop it

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->
The distinctive icons for backing tracks and for the stem tracks is open. We
can either use icons or simply "B" and "S" style icons.

The way we represent the temporary files inside the track repository is open.
