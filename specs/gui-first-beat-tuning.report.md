# Report: gui-first-beat-tuning

Status: **DONE**

Branch `spec/gui-first-beat-tuning`, base `021f128`. Not pushed or merged.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| First bar + Right: drag readout shown, gone ~1.5 s after the last key | `WaveformView.test.ts` "keyboard readout"; GUI `player_first_beat_tuning`, `editor_first_beat` (screenshots) | PASS |
| A detected change moved with the keys shows its drag readout | `WaveformView.test.ts` (Editor and Player modes) | PASS |
| `+10` + Enter in section 1: 10/64 beat after the found start, field `+10` and its time | `ControlBar.test.ts`; GUI step 3 (`player.json` start 1278.125, `found_ms` 1200) | PASS |
| Beyond the limits: stops at 0:00 / one beat before the next section, field shows the applied value | `waveform-geometry.test.ts` `clampMoveSteps`; `player-state.test.ts`; `ControlBar.test.ts`; GUI step 4 | PASS |
| Not a whole number: message, nothing changes | `section-meta.test.ts`; `ControlBar.test.ts` (edge texts); GUI step 5 | PASS |
| Moved by keys or Find start: the field follows | `ControlBar.test.ts` | PASS |
| Detected section, `-8`: only that bar moves, field `-8` | `ControlBar.test.ts`; GUI step 7 | PASS |
| Playhead into section 2: the field shows section 2's offset | `ControlBar.test.ts`; GUI screenshot | PASS |
| Section 2: Find start disabled, field editable | `ControlBar.test.ts`; GUI screenshot | PASS |
| Middle of section 2: Next to section 3's start, Prev to the first beat | `ControlBar.test.ts`; GUI steps 8-9 | PASS |
| Next disabled in the last section, Prev in the first | `ControlBar.test.ts`; GUI screenshots | PASS |
| While playing, Next behaves like \|◀ (no count-in, owner answer) | `player-state.test.ts` (same payload); GUI step 10 (captured audio) | PASS |
| 1280x800: no overflow, Loops beside Sections | GUI `assert_tuning_layout`: `overflow=0 loops_beside=1 sections_w=496` | PASS |

Suites on `134d960`: `npm test` passes, and the whole GUI suite passes (all 9 binaries).

## Task log

| Step | By | Commit |
|---|---|---|
| plan | architect | d0282ae |
| task 1: offset text helpers | local model | 6c1a53a |
| tasks 2-5: geometry helpers, keyboard readout, field and Prev / Next state, Sections pane | implementer | d07eb5f |
| tasks 6-8: GUI tests, re-measured coordinates, docs | implementer | 04b5651 |
| tester: 8 acceptance tests, no defects | tester | c1d418c |
| fix round 1 | implementer | 134d960 |

The local model did task 1. Its task 2 code matched the plan, but it stopped without writing the tests, so
the result was discarded and the implementer did it.

## Review

APPROVED, no blockers or majors; the fix round was reviewed again. Fix round 1:
- the keyboard readout goes away when Up/Down selects another bar;
- leaving the field after a refused Enter cancels the edit instead of showing the error again;
- the field's accessible name is its visible label "Start";
- comments: the frontend clamp mirrors `tempo_map.rs` and must change with it.

Left as is: the keyboard and drag readouts aren't announced to screen readers (`aria-live`).

## Deviations from the plan

- The tablature placeholder shrank by about 18 px for the new Start row, so the waveform and the
  Sections / Loops panes sit higher. All GUI test coordinates were measured again.
- The GUI tests' `typ` helper now passes `--` to xdotool, so text starting with `-` can be typed.
- On a typed (not detected) section, the field counts from the previous section's start, as the drag
  readout does, so it can show large values (e.g. `+154` for a section 1.2 s after the previous one).

## Follow-ups

- Full parallel GUI runs: the first test of `gui_import_e2e`, `gui_player_acceptance_e2e` and
  `acceptance_stem_extraction_gui` sometimes times out waiting 30 s for the app to report it is ready.
  Each passes alone, and the final run passed. It's about starting nine apps at once, not this feature.
- With the library tree shown, the Player at 1280x800 still has Loops below Sections (as before).

## How to try it

Laptop: pull, `npm run build:app`, install as before. No file format change.

## Manual checks (laptop)

- [ ] **Keyboard readout:** in the Player, click the waveform in the first section and press Right a few
  times. The readout shows "+N/64 beat (time) from the found start" and disappears about 1.5 s after the
  last press. The same in the Editor's waveform.
- [ ] **Typed offset:** with the playhead in the first section, type `+10` in "Start" and press Enter. The
  first bar moves a little later and the field reads `+10` with its time. Play with Count In and listen.
- [ ] **Limits and bad text:** `-9999` stops the bar at 0:00 and the field shows the applied value; `abc`
  shows a message and nothing moves.
- [ ] **Find start:** the field goes back to `0`.
- [ ] **Other sections:** with Next, go to a later section. "Find start" is greyed out with a hint; the field
  shows that section's offset. Type `-8`: only that bar moves.
- [ ] **Prev / Next:** Next goes to the next section's start; Prev from the middle of a section goes to the
  previous section's start (from section 2: the first beat). Next is disabled in the last section, Prev in
  the first.
- [ ] **While playing:** Next into a section behaves like |◀ in that section (no count-in).
- [ ] **Layout:** at 1280x800 with the tree folded, Sections and Loops are side by side, nothing scrolls.

Roadmap: tick `gui-first-beat-tuning.md` in `specs/overview.md` once the manual checks pass.
