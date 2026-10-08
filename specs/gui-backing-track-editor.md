# Backing Track Assembly

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Goal is to create a backing track out of a collection of stems extracted out
of a music track by the stem extraction feature found in the import page.

Musician switches to the Editor view, manipulate the stem track and then they
will end-up with a backing track next to the existing stems.

The Editor view has two vertical main sections. Left side shows the same tree
view from the Library view. Right hand pane is the editor itself.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. Reuse the library view's tree view
2. Editor pane activates when the selected track has stems previously
   extracted
3. Editor view presents the stems vertically, in horizontal lanes
4. One stem lane has the, from left to right: stem label (name), "play" button,
   "unmute" checkbox, "volume" horizontal slider
5. We present 5 stem lanes but keep in mind that tracks can have more than 5
   stems in the future, so we actually show a view having as many lanes as the
   track has
6. Under the stems view we have the "Mix" lane
7. "Mix" lane has, from left to right: "Mix" label, "Play/Pause" button, "Stop" button, time slider, current time in "M:S:SS" meaning "minute:second:10th second" being and editable field that updates along with the slider upon track play, "Save"
8. The time slider and the current time edit box keep the playing position for
   the mix but also for the stems play position

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given library tree view has entries, when user selects track without
  stems, then the editor pane stays inactive
- [ ] Given library tree view has entries, when user selects track with stems,
  then the editor pane becomes active
- [ ] Given stem lane "Play" button is clicked, then the "unmute" check box
  for that stem lane becomes checked and the mix "Play" button gets clicked
- [ ] Given at least a stem "unmute" checkbox is checked, then Mix Play button
  is enabled
- [ ] Given the "Play" button is clicked, when at least a stem unmute is
  checked, then start playing the combined stems
- [ ] Given the play started or resumed, the time slider gets updated along
  with the curent time text box
- [ ] Given the play started, then "Play" button becomes "Pause"
- [ ] Given the play started, then "Stop" button becomes enabled
- [ ] Given the Pause button is clicked, then plyback pauses at the current
  position
- [ ] Given the Pause button is clicked, then "Pause" becomes "Play"
- [ ] Given the user edits the time position edit box, when the value is
  withing the track duration range, then current play position jumps to that
  position
- [ ] Given the user turns mouse wheel up, when the mouse cursor is over the
  time position edit box, then the time position gets adjusted towards the
  beginning of the track by .1s per wheel step
- [ ] Given the user turns mouse wheel down, when the mouse cursor is over the
  time position edit box, then the time position gets adjusted towards the
  end of the track by .1s per wheel step
- [ ] Given the time position is being adjusted, when play is going on, then
  stop playing and resume it from the new position after .3s
- [ ] Given the time position is being adjusted, when we are in the .3s delay
  window, then reset that delay window to .3s
- [ ] Given the Save button is clicked, when at least a stem is unmuted,
  create the backing track file out of the configured mix
- [ ] Given the lane volume slider is adjested, when the stem lane is unmuted,
  then use that volume's adjustment value to contribute to the mix


## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->
