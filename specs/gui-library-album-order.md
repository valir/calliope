# Library: album and track order, deleting tracks and albums

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Show the tracks of an album in the publisher's order and a band's albums in
chronological order, instead of by name, with the order set by reordering
them in the Library tree, and let the user delete a single track or a whole
album right from the tree.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `gui-tracks-repository.md` built the Library view: a band / album / track
  tree, sorted alphabetically (case-insensitive) at every level, with
  keyboard navigation (arrows, Home/End, Enter/Space), Collapse all, Expand
  all and the search box, and the track pane on the right.
- The track pane already has a Delete button for the selected track. It asks
  for confirmation and moves the track folder to the repository's `trash/`
  folder. There is no way to delete an album, and nothing in the tree
  itself deletes.
- Track metadata is `track.json`, schema version 2; older versions are
  migrated in memory when read.
- Imports (`gui-stem-extraction.md`, `gui-existing-track-import.md`,
  `gui-import-local-batch.md`) create tracks with the file's own tags; a
  whole album folder can now be imported in one go.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. Each track can have a position in its album (1, 2, 3, ...), stored in its
   metadata and kept when the track is edited, exported or the app restarts.
2. Under an album, the tree shows the tracks with a position first, in
   position order, then the tracks without one, alphabetically as today.
   Bands stay alphabetical.
3. The tree shows the position in front of the title (e.g. "3. Hold On")
   when the track has one.
4. Tracks can be reordered inside their album by drag and drop in the tree:
   dropping a track between two others of the same album moves it there,
   with a visible drop marker while dragging. Dropping it anywhere else
   (another album, a band, outside the tree) does nothing.
5. Tracks can be reordered with the keyboard: with a track focused in the
   tree, Alt+Up / Alt+Down moves it one place up or down in its album; the
   track keeps the focus.
6. After any reorder, every track of that album gets a position matching
   the shown order (1..n, no gaps), and the new order is saved at once.
7. A band's albums can be reordered the same way (drag and drop between the
   band's other albums, Alt+Up / Alt+Down on a focused album), so they can
   be put in chronological order. Albums with a position come first, in
   position order, then those without one, alphabetically; the "no album"
   group is always last. Albums show no number. Like tracks, after a
   reorder every album of that band gets a position (1..n), saved at once,
   and an album cannot be dropped under another band. The album's tracks
   keep their own order.
8. Reordering is not available while the search box filters the tree or
   while the track pane is in edit mode.
9. Imports set a track's position from the source file's track-number tag
   when it has one (e.g. "3" or "3/12" gives 3); a track without the tag
   gets no position. An import never changes the positions of tracks
   already in the album.
10. The position is shown read-only in the track pane; the order is changed
    only in the tree.
11. A single track never moves to another album or band. Instead, changing
    the band or the album field in the track pane's edit mode renames that
    band or album for all of its tracks together: after Save, a
    confirmation names the old and the new name and the number of tracks
    affected, and then every one of those tracks is updated. The tracks
    keep their positions, and a renamed album keeps its place among the
    band's albums.
12. Deleting a track from the tree: with a track selected or focused in the
   tree, a Delete button above the tree (next to Collapse all / Expand all)
   and the Delete key ask for confirmation and then delete it the same way
   as the track pane's Delete (moved to the trash folder).
13. Deleting an album: with an album selected or focused in the tree, the
   same Delete button and key ask for confirmation, naming the band, the
   album and the number of tracks, and then delete every track of that
   album the same way.
14. If some tracks of an album cannot be deleted, the others are still
    deleted and the user is told which ones failed and why.
15. After a delete, the tree shows the remaining tracks with the focus on a
    neighbouring item; an album or band with no tracks left disappears.
16. Bands cannot be deleted as a whole, and the "no album" group of a band
    is not an album: it cannot be reordered or deleted as a group (its
    tracks can still be deleted one by one).
17. Reorders and deletes are written to the log.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- `track.json` files written by earlier versions keep working: they load
  with no track or album positions and show in alphabetical order.
- Tests use temporary repositories only, never the user's real repository.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

Moving a single track to another album or band (by drag and drop or by
editing).
Reordering bands. Showing album years. Disc numbers for multi-disc albums. Emptying or browsing the
trash folder, and undo. Deleting from the Player, the Editor or playlists.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given an album with tracks "A", "B", "C" and no positions, when the
  Library opens, then they are shown alphabetically without numbers
- [ ] Given that album, when the user drags "C" above "A", then the tree
  shows "1. C", "2. A", "3. B", also after the app is restarted
- [ ] Given "2. A" is focused, when the user presses Alt+Up, then
  the tree shows "1. A", "2. C", "3. B" and "A" keeps the focus
- [ ] Given "1. A" is focused, when the user presses Alt+Up, then
  nothing changes
- [ ] Given a track is dragged onto another album, then nothing changes
- [ ] Given an album with positioned tracks, when another track is added to
  it without a position, then it is shown after the positioned ones
- [ ] Given a band with albums "Alpha", "Beta", "Gamma" and no album
  positions, when the user drags "Gamma" above "Alpha", then the band shows
  "Gamma", "Alpha", "Beta", also after a restart, and each album's tracks
  keep their order
- [ ] Given "Alpha" is focused under that band, when the user presses
  Alt+Down, then the band shows "Gamma", "Beta", "Alpha"
- [ ] Given an album is dragged onto another band, then nothing changes
- [ ] Given a band with a "no album" group, then that group stays last
  whatever the album order
- [ ] Given a directory import of files tagged with track numbers 2, 1 and 3,
  then the album shows them as "1. ...", "2. ...", "3. ..." in tag order
- [ ] Given the track pane in edit mode, then the position is shown
  read-only
- [ ] Given an album "Masterof Puppets" with 8 tracks, when the user edits
  one track's album to "Master of Puppets" and saves, then the confirmation
  names both names and "8 tracks", and after confirming all 8 tracks are
  under "Master of Puppets" in the same order
- [ ] Given a band "Metalica" with 2 albums, when the user edits one track's
  band to "Metallica" and saves, then all tracks of both albums move under
  "Metallica" with their album and track order kept
- [ ] Given that confirmation, when the user cancels, then nothing is
  saved and the pane stays in edit mode
- [ ] Given text in the search box, then dragging and the move keys do not
  reorder
- [ ] Given a track selected in the tree, when the user presses Delete (key
  or button) and confirms, then the track is moved to the trash folder and
  disappears from the tree
- [ ] Given an album of 4 tracks selected in the tree, when the user presses
  Delete, then the confirmation names the band, the album and "4 tracks";
  after confirming, all 4 are moved to the trash folder and the album
  disappears
- [ ] Given the confirmation is shown, when the user cancels, then nothing
  is deleted
- [ ] Given an album delete where one track folder cannot be moved, then the
  other tracks are deleted and the failed one is reported with its reason
- [ ] Given a band is focused, then Delete is disabled
- [ ] Given a track.json from the previous version, when the Library opens,
  then it loads unchanged, with no position

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

Answered by the owner (2026-10-20): Alt+Up / Alt+Down move the focused item;
imports use the source's track-number tag when it has one; the order changes
only in the tree; a single track cannot move to another band or album, and
editing the band or album renames it for all its tracks; band albums can be
reordered too (for chronology).

1. Renaming a band or album to a name that already exists (e.g. "Metalica"
   to an existing "Metallica"): merge them? (Proposed: yes, after the same
   confirmation, which then says they will be merged; the renamed albums or
   tracks keep their order and come after the existing ones.)
2. Changing only the upper/lower case of a name ("metallica" to
   "Metallica") is also a rename of all its tracks. (Proposed: yes.)
3. A track in a band's "no album" group isn't in an album yet. Can setting
   its album field put just that one track into an album? (Proposed: yes,
   only that track, and it joins at the end of that album. Clearing the
   album or band name of a track in an album is refused, since it would
   take the track out of its album.)
