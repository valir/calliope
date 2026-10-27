# Report: gui-section-bpm-detect

Status: **DONE**

Branch `spec/gui-section-bpm-detect`, base `b7e07ee`. Not pushed or merged.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| 100 BPM (4/4), playhead 10 s in: the 7.2 s from the playhead are sent | `ac01`; `player_ipc_tests` (158 760 frames equal to the backing slice); GUI `player_section_bpm_detect` | PASS |
| Playhead 3 s before the section's end: only those 3 s | `ac02`; `section_bpm_tests` `piece` | PASS |
| Less than 1 bar before the end: message, server not called | `ac03`, `ac13` (3/4), `ac14` (track end); `player_tests` refusals; GUI step | PASS |
| The selected source is sent, not the backing | `ac04` (Guitar stem selected); `player_ipc_tests` | PASS |
| 93.4 BPM applied, starts and other sections unchanged, saved in `player.json` | `ac05`; `player_tests`; GUI step | PASS |
| Status shows old / new BPM and the range; about twice: points to ×2 / ÷2 | `ac06`, `ac19` (thresholds); `bpm-format.test.ts`; `ControlBar.test.ts`; GUI screenshots | PASS |
| Server down or no steady beat: message, BPM kept | `ac07`, `ac20` (too few beats, tracker failure); `section_bpm_tests` | PASS |
| Cancel: BPM kept, button free again | `ac08`; `player-state.test.ts`; GUI step (button reads Cancel while running) | PASS |
| Section removed / edited or another track opened: result not applied | `ac09`, `ac15`; `player_tests` | PASS |
| Playing with the Click on: clicks follow the new BPM, playback goes on | `ac10`; `player_tests`; GUI step (captured audio: 642 ms gaps, then 500 ms) | PASS |
| 1280x800: no overflow, Loops beside Sections | GUI layout line: `overflow=0 loops_beside=1 sections_w=496` | PASS |

Also tested (tester): playhead before an unset first beat (piece from the section's start), a section shown away from
the playhead (piece from its start, owner answer), 3/4 bars, the last section ending at the track end, a running
detection visible in the snapshot after a webview reload, all refusals start nothing.

Suites on `644560d`: `npm test` passes; the whole GUI suite passes (one unrelated Import test timed out under the
parallel load and passed alone; same for four app-startup timeouts on the first run).

## Task log

| Step | By | Commit |
|---|---|---|
| plan (with the owner's change: the button is never disabled for the playhead) | architect | 3f442da |
| task 1: stub tracker tempo modes, input copy | local model (tidied by me) | c9fd29d |
| tasks 2-4: `piece_bpm`, shared upload helper, `section_bpm.rs` | implementer | aecedb4 |
| tasks 5-6: the run in the Player session, IPC commands | implementer | 0e74b2a |
| task 7: status texts | local model (duplicate test block removed by me) | d4ad541 |
| tasks 8-9: player state, Detect BPM button | implementer | df867e7 |
| tasks 10-12: acceptance tests, GUI test and positions, docs | implementer | f1472e3 |
| tester: 10 more acceptance tests, no defects | tester | e566ace |
| fix round 1 | implementer | 644560d |

The local model did tasks 1 and 7. Both were correct but untidy (task 1 dropped a README sentence and left a scratch
folder in the repo; task 7 wrote its tests twice); I cleaned them up before committing.

## Review

APPROVED, no blockers or majors; APPROVED again after fix round 1. Fix round 1:
- a result that arrives before the start reply (e.g. a server refusing connections at once) is no longer lost, so the
  pane can't stay on "Detecting..." / Cancel;
- a section at BPM 0 is refused with "Type a BPM first" instead of the "Less than 1 bar" message;
- one bar of 3/4 (3 beats) is enough to measure; 4/4 still needs 4.

Left as is (minor): if the track changes while a start is in flight, a quick second click on the new track could send a
second request; the backend refuses it as busy. Events of a finished run are emitted under the manager lock, like the
other Player events.

## Deviations from the plan

- **Owner change:** the button acts on the section the pane shows and is never disabled because the playhead is
  elsewhere; then the piece starts at that section's start.
- **Message line:** a long message (e.g. the too-short one) widened the Sections pane and pushed Loops below it. The
  message line now truncates with "…" and keeps the full text as its tooltip.
- The Start row's unit reads "/64" (was "/64 beat") and its time is smaller, to make room; Sections is exactly 496 px
  wide at 1280x800. Find start moved left; no other GUI coordinate changed.
- The idle button's accessible name is "Detect the BPM of this section from 3 bars".

## Follow-ups

- Full parallel GUI runs still have first-test startup timeouts now and then (known, not this feature).
- The Start row is at its width limit at 1280x800: one more control there will need another layout.

## How to try it

Laptop: pull, `npm run build:app`, install as before. No file format change. No server change: calliope-stems on
archserver stays as it is.

## Manual checks (laptop, real server)

- [ ] **Basic detection:** open a track whose section BPM is off, put the playhead a few bars into the section where the
  drums play, press "Detect BPM" on the Start row. The message reads "Detecting the BPM from a – b..." and the button
  "Cancel"; a few seconds later the section's BPM changes and the message reads "BPM old → new, detected from a – b"
  (about 3 bars from the playhead).
- [ ] **Listen:** turn the section's Click on and play it; the clicks should sit on the beat across the section.
- [ ] **While playing:** detect during playback; playback goes on and the clicks switch tempo when the result arrives.
- [ ] **Source:** choose a stem in the dropdown (e.g. drums) and detect again; compare with the backing.
- [ ] **Near the end:** less than a bar before the next section, Detect shows "Less than 1 bar ..." at once.
- [ ] **Octave:** a result about twice or half the old BPM ends with "try ÷2" / "try ×2"; those buttons fix it.
- [ ] **Cancel:** start a detection while the server is busy (e.g. during a stem extraction), press Cancel: the BPM
  stays and the button reads Detect BPM again.
- [ ] **Server down:** with calliope-stems stopped, Detect shows "Cannot reach the edge-AI server at ..." and nothing
  changes.
- [ ] **Saved:** reopen the track or restart the app; the detected BPM is still there.
- [ ] **Layout:** at 1280x800 with the tree folded, the Start row is one line and Loops is beside Sections.

Roadmap: add/tick `gui-section-bpm-detect.md` in `specs/overview.md` once the manual checks pass.
