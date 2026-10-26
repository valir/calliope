# First beat tuning and section navigation

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Make fine-tuning the first beat easier: show the offset while moving a bar
with the keys, let a section's start offset be typed in the Sections pane, keep
"Find start" to the first section, and add buttons that step the playhead from section to
section.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `gui-first-beat.md` placed the first section's start bar where the music
  starts, with a "Find start" button in the Player's Sections pane. Section
  start bars move by mouse drag or with the keys (1/64 beat, Shift = 1 beat).
- While dragging, a readout on the waveform shows the offset, e.g.
  "+10/64 beat (+0:00.078) from the found start" for the first bar, and the
  offset from where the change was detected (or, for a typed change, from the
  previous section's start) for the others. Keyboard moves show nothing.
- The Sections pane acts on the section under the playhead. Its |◀ and ▶|
  buttons jump to the start and the end of that section
  (`gui-player-tempo-sections.md`).
- The owner's tests (2026-10-26): the first-beat count-in is close; the
  keyboard needs the same feedback as the mouse.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. **Readout for keyboard moves:** moving a section start bar with the keys,
   in the Player and the Editor, shows the same readout as a drag, at the same
   place. It stays up while keys are pressed and disappears about 1.5 s after
   the last key press, or when the waveform loses focus.
2. **Start offset field:** the Sections pane has a field holding the start
   offset of the section under the playhead, in 1/64-beat steps (e.g. `+10`),
   with the time next to it (e.g. "+0:00.078"). The offset is counted from the
   same point as the drag readout: the found start for the first section, the
   detected position for a detected change, the previous section's start for a
   typed one. It is editable in every section. Typing a value and confirming
   (Enter or leaving the field) moves that section's start bar to that offset,
   clamped like a drag (the first section: 0:00 up to one beat before the next
   section; the others: their existing drag limits); the field then shows the
   value actually applied. Invalid text is refused with a message and changes
   nothing. Moving the bar by mouse, keys or "Find start" updates the field,
   and moving the playhead into another section shows that section's offset.
3. **Find start only in the first section:** outside the first section the
   button is disabled, with a hint that it works in the first section.
4. **Prev / Next section buttons** in the Sections pane:
   1. **Next section** puts the playhead at the start of the section after the
      one under the playhead. Disabled in the last section.
   2. **Prev section** puts the playhead at the start of the section before the
      one under the playhead, wherever the playhead is inside its section.
      Disabled in the first section. The start of the first section is the
      first beat.
   3. While playing, they behave like the existing |◀ button (the same
      count-in rules for a section with Count In on).
5. The Player still fits at 1280x800 with Loops beside Sections; the new
   controls may make the row wrap only if the layout already allows it.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- Tests use temporary repositories only, never the user's real repository,
  real devices or the real system dialogs.
- No `symphonia`.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

A start offset field in the Editor (the Editor keeps the keys, the drag and
the new keyboard readout). Keyboard shortcuts for Prev/Next section.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given the first bar selected on the waveform, when Right is pressed,
  then the readout shows "+1/64 beat (...) from the found start" at the same
  place as during a drag, and it is gone about 1.5 s after the last key press
- [ ] Given a detected section change selected, when it is moved with the
  keys, then the same readout as its drag is shown
- [ ] Given the playhead in the first section, when `+10` is typed in the
  first-beat field and Enter pressed, then the first beat is 10/64 beat after
  the found start and the field shows `+10` and its time
- [ ] Given a value beyond the limits, when it is confirmed, then the first beat
  stops at 0:00 / one beat before the next section and the field shows the
  applied value
- [ ] Given text that is not a whole number, when it is confirmed, then a
  message says so and nothing changes
- [ ] Given the first bar moved by keys or "Find start", then the field shows
  the new offset
- [ ] Given the playhead in a section with a detected change, when `-8` is
  typed in the field and confirmed, then that section's start bar is 8/64 beat
  before the detected position, the other sections are unchanged, and the
  field shows `-8`
- [ ] Given the playhead moved from section 1 to section 2, then the field
  shows section 2's offset
- [ ] Given the playhead in the second section, then "Find start" is disabled
  and the field is editable
- [ ] Given three sections and the playhead in the middle of section 2, when
  Next section is pressed, then the playhead is at the start of section 3;
  when Prev section is pressed from the middle of section 2, then it is at the
  first beat
- [ ] Given the playhead in the last section, then Next section is disabled;
  in the first section, Prev section is disabled
- [ ] Given playback with Count In on in the target section, when Next
  section is pressed, then it counts in as |◀ does
- [ ] Given the Player at 1280x800, then the layout has no overflow and Loops
  stays beside Sections

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

Answered by the owner (2026-10-26): "Find start" is disabled, not hidden,
outside the first section; the field holds 1/64-beat steps from the found
start (the other sections: from their drag reference) with the time next to
it, editable in every section; Prev section always goes to the previous
section's start; the keyboard readout fades about 1.5 s after the last key.
