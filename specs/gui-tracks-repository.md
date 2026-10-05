# Track Repository

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Manages the tracks known to Calliope. Guitar player uses this to play the
tracks and to create playlists. Guitar player imports here the existing
backing tracks or the tracks made out of the music tracks.

The repository is a disk directory containing audio files, guitar tablature
files and per file metadata.

For one audio file, which always is supposed to be a backing track:
* associate metadata
* allow associating one or more tablature files

File format should default to mp3 - we do not focus on lossless, but on
efficieny and ease of handling of these fils. The quality should be on the
higher side though. In the future we'll add lossless formats though.

Metadata per file should use JSON format preferably and it will see structural
enhancements on our way of implementing Calliope. So this format should
contain a version number in it so we could correctly serialize it.


## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

Storage should be able to handle stems extracted by the stem extraction tools
available in the ~/edge-ai directory.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1.  Initial structure for the metadata:
* track id
* band name
* album name
* track title
* composed by (list of authors)
* composed year if any
* downloaded from link if applicable
* copyright of the track if any
* filename for the main audio backtrack
* tablature files
* imported date
* modified date
2. root directory holding the repository can be configured in the GUI and defaults to
   ~/.local/share/calliope/ (e.g. XDG standard location)
3. repository root directory contains a `tracks` directory organizing the
   tracks
4. each track has its own directory containing the corresponding files
   mentioned in the metadata
5. filenames in the metadata are relative to the track directory
6. track directory names do not need to be human readable or be related to the
   actual track metadata information
7. The GUI manages the repository data in the Library view
8. The Library view is split in two columns
9. Left column shows the existing track list and organizes the tracks in a
   tree view, grouping band/album/track name ; above that list calliope
   provides tool area containing:
   - collapse all button
   - expand all button
   - search box
   - clear search box button
10. right column shows track pane
11. track pane displays track information out of the track metadata, initially
    in R/O mode ; a button press switches the pane in edit mode. The track ID
    is never editable and is unique per track
11. track pane allows the operations:
   - edit mode
   - save changes
   - export track
   - delete track
12. track filtering behavior: list contents is being filtered with each
    character typed in the search box; search is case insitive and we'd like
    to have "fuzzy search"
13. track pane contains a sub-panel allowing for tablature management for
    the current track as we can associate several tablatures for a single
    track. We have a list sized for 3 or 4 visible tablature names, with
    scrollbar when more than the visible lines are present. Under that, we
    have "Add", "Update", "Export", "Remove" buttons. This pane has a label
    "Tablature Files"; this pane becomes active only when pane is in edit mode

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given the Library tab is active, when repository contains tracks, then the
  track list is being populated in a collapsed initial form
- [ ] Given the Library tab is active, when repository contains tracks, then
  the track list is being poluated with with a three level tree list: band
  name, album, track name; each level is being sorted alphabetically case
  insensitive
- [ ] Given track list contains tracks, when expand all button is clicked,
then all list entries are expanded
- [ ] Given track list contains tracks, when collapse all button is clicked,
  then all list entries are collapsed
- [ ] Given track list contains tracks, when we enter text in the search box,
  then the list is filtered in respect with requirement 12
- [ ] Given track list is empty, when the Library tab is active, the track info
  pane has the fields visible but inactive
- [ ] Given track list has no item selected, when the Library tab is active, the track info
  pane has the fields visible but inactive
- [ ] Given track list is not empty, when a track is selected, then the track
  pane shows the metadata information and shows the manipulation buttons
  which are "Edit", "Save", "Export", "Delete" ; manipulations are placed in
  the lower part of the track information pane ; they are all active except
  "Save" button
- [ ] Given a track metadata is displayed, when Edit button is clicked, then:
  - metadata fields are switched to edit mode except for the track id
  - the "Edit", "Export" and "Delete" buttons become inactive
  - the "Save" button become active
  - the "Tablature Files" pane becomes active
- [ ] Given the track pane is in edit mode, when Save button is clicked, then:
  - data is being transfered from the edited fields to the metadata file on disk
  - the modified date metadata field has the save timestamp value
  - the pane returns to the read-only mode
  - the "Save" button becomes inactive
  - the "Edit", "Export" and "Delete" buttons return to active
- [ ] Given the pane is in edit mode, when the metadata has tablature files for
  the current track, then the "Tablature Files" displays them
- [ ] Given the pane is in edit mode, when the metadata has not tablature
  files for the current track, then only the "Add" button is active
- [ ] Given the pane is in edit mode, when a tablature is selected, then the
  "Update", "Export" and "Delete" buttons transition to active
- [ ] Given the pane is in edit mode, when the "Add" button is clicked, a
  browse for file dialog is being popped up, filtering for tablature files
- [ ] Given the browse for tablature file is displayed, when a tablature file
  is selected and dialog is confirmed, then:
  - the new tablature file name is added to the list, stripped of the path
    part and selected
  - the "Delete" button becomes active
- [ ] Given tablature file has been added to the tablature list in edit mode,
  when Save button is clicked, then:
  - the file is copied in the repository
  - medadata tablature list for the track is updated with the tablature file
    name
  - metadata record timestamp is updated
- [ ] Given the pane is in edit mode, when the "Delete" button is clicked,
  then a confirmation dialog is shown saying "Delete backing track ..." with
  the dots filled in with the actual tablature name and the "Yes"/"No" buttons
- [ ] Given the tablature delete confirmation dialog is displayed, when the
  "Yes" button is selected, then the tablature is removed from the list but
  not yet from the actual metadata
- [ ] Given tablature file has been removed from the tablature list in edit mode,
  when Save button is clicked, then:
  - the file is removed from the repository
  - metadata tablature list for the track is updated by removing that
    tablature name
  - metadata record timestamp is updated


## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->
* The track ID is not yet decided upon.
* The `tracks` directory contains per track directory but the naming
  conventions for the per-track directories is not defined. Also, we also did
  not define if we need intermediate short name directories to group track
  directories by, say, first letter of the track id

