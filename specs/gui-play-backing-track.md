# Play a backing track

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

This function effectively help muzic player train using the previously
prepared backing track. The application provides a set of controls allowing
for specific parts of the track to be loop played, or slowed down. Also,
    accessories like BPM detection and metronome are attached to the track.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. The Library tree is also present in this view, but it can be reduced to the
   left analoguous to the left-side views list, using the shortcut CTRL+L
2. When a track is selected in the library tree, the right side of the view
   would show the player
3. The player saves the state of the different controls attached to the track
   and restores them when track is selected, no save button is needed to
   perform settings change on disk
4. the top of the view shows a dropdown list containing the track mix and stem
   names. upon first visit of a track, dropdown automatically selects the
   backing track (the mix) of the track if available. If not, it selects the
   first available stem. Later on, when user changes selection in this
   dropdown, we persist the current selection on disk next to the track
5. under the dropdown comes the actual player pane
6. player pane provides a central empty space for guitar tablature display -
   will be specified in the future
7. under the tablature view (which how is empty) we have the playback control
   view
8. The playback control view is divided in two horizontal lanes: the waveform
   view and the actual playback control bar
8. playback control bar has several sections: BPM, Metronome, Playback, Loops
9. BPM section has a text edit for BPM display/entry labeled "BPM" followed by
   four buttons "-", "+", "Reset", "Detect" and a "V" marker
10. Metronome section has push button "On/Off", "Count In"
11. Playback has "Play/Pause", slider, "Rew", "FF", position textbox showing
    position in "M:SS.m" format, editable
12. Loops has groups (several buttons: "Prev", "+", section number read-only
    text box, "-", "Next")(">" button, "bar1" text box, small slider, "bar2"
    textbox, "<" button)("Play/Stop" button, "Loop On" push button,
    "Count-In" push button)
13. All the player controls apply to the selected mix or stem from top
    dropdown list; settings are persisted as soon as they are adjusted and are
    restored if available upon reload of track
14. BPM requirements:
    - each track should have a BPM attached, defaulting to 0
    - if BPM is 0, then the loop controls operate in timestamp mode
    - tracks can have sections using different tempos
    - tracks having multiple tempos check the "V" marker
    - the tempo can be manually introduced and it will tak effect from the
      current playing position to the end of the track or to the next tempo
      change
    - the tempo can be auto detected, including multiple tempos per sections;
      if needed, the "calliope-stems" edge-ai server will be used to trigger
          the appropriate model able to perform this task
    - the "-" and "+" buttons decrease or increase existing BPM with 5 units;
      this is and the "Reset" button resets BPM to the correct BPM for the
      current playing position
    - BPM adjustement in respect to the correct BPM would effectively produce
      slowdown or speed-up of the playback speed wile preserving overall pitch
      of the mix or track
15. Metronome requirements:
    - plays an audible tick according to the current BPM when it's "On"
    - count-in becomes available when BPM is not zero
    - when active, count-in alter the play function: first the metronome
      produces 3 ticks and then playback starts on the 4th beat
    - the 4th tick is not audible
    - the count-in is also used when playing loops if "On"
    - if a loop is active, only play count-in for when starting the section
      play then when returning to the beginning of section, count-in is not
      being played except for the case the "Count-In" push button of that loop
      is "On"
16. Loops requirements
    - Several loops can be associated to the current track or stem
    - "+" creates a new loop section, "-" removes current loop section
    - "prev"/"next" buttons navigate to the previous or next loop section
    - loop navigation buttons instantly jump the current playing position to
      the loop beginning position
        - timestamp mode saves absolute timestamps
        - BPM mode only saves the closest time division, up to 1/64th
    - loops operate in 2 modes depending on available BPM or not
    - the ">" button saves/overwrites current loop start position; in BPM mode
      adjustement is made to the closest 1/64th position, forward or
      backwards depending on wich one is closest
    - the "<" button saves/overwirtes current loop end position, same way as
      the ">" button
    - the slider shows current position inside the loop, as a visual cue; it
      is read-only for the user
    - play button on a loop immediately jumps to loop start position then
      starts playing that section, eventually preceded by metronome count-in
      of "Count In" is active for this loop
    - there is no "pause" function for a loop, only play or stop playing
    - the overall BPM adjustments affect also loop playback
17. Waveform view requirements:
    - it would stretch horizontally
    - it draws the track waveforms so user has visual cues about different
      sections in sound intensity respect
    - it has vertical marks corresponding to the loop sections; using accent
      color then an accent color horizontal bar joining the start/end
      positions as a visual cue for that section

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

The other views should be left untouched, we only focus on the playback view
here.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given …, when …, then …
- [ ] Given …, when …, then …

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->
