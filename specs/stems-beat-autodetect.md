# Stems beat autodetection

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Help musician setup the metronome and the count-in for the backing track by
using beat autodetection. Main operation takes place in the Editor view.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. the editor view receives inside the mix pane the waveform view the player
   has
2. the mix pane has a new button "Autodetect BPM"
3. the waveform view will be able to visually identify the BPM sections by
   separating the lane with vertical accent colors and displaying the
   corresponding BPM to the right of the section start bar as so: far left has
   no bar, as it's the natural view edge - there only place the detected start
   BPM; if any BPM changes are occurring during the rest of the track then
   place a bar at the right moment then follow it by the new BPM display;
   repeat until all BPM changes are being displayed.
4. "the right moment" in requirement 3. means an alignment in terms of BPM of
   the previous section. As BPM changes only can occur when previous beat is
   done. We choose a granularity of 1/64 of the BPM, but we'll enable user
   fine adjustement of this moment, in 1/64 steps

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

The player view only needs to be update in order to remove the "Autodetect"
button. All other changes in the Player view or any other view should be
confirmed first. Tests should stay focused on the current function.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given the mix stem composition changes, then the waveform view is being
  updated without blocking the GUI after 0.3s delay
- [ ] Given the mix stem composition changes, when we are in the 0.3s delay
  after previous change, then restart the 0.3s waiting before updating the
  waveform
- [ ] Given the mix has at least a stem in it, then activate the "Autodetect"
  button
- [ ] Given the "Autodetect" is clicked, then the autodetect algorithm is
  being run on the mix; if this requires AI processing, then delegate task to
  calliope-stems
- [ ] Given the song has several BPM changes, then the metadata will be
  updated accordingly and also the waveform
- [ ] Given BPM was successfully detected, then save the information along the
  mix track
- [ ] Given the song has multiple BPMs, when the user drags the BPM section
  start, then allow that bar movement left or right in increments of 1/64 of
  the left section's BPM but for no more than 4 beats

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

calliope-stems AI processing is open for discussion if BPM detection can be
done using AI.

