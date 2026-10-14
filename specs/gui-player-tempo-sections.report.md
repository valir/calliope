# Report: Player tempo sections editing

**Status: DONE.** All 7 criteria and the 3 the plan added pass. The reviewer approved after one fix round, and the tester re-verified the fixes with no new defects.

- Branch `spec/gui-player-tempo-sections`, base `0bd82c3`. Not pushed or merged.
- Spec: `specs/gui-player-tempo-sections.md`. Plan: `specs/gui-player-tempo-sections.plan.md`. Log: `specs/gui-player-tempo-sections.log.md`.

## What you get

**Editing in the Player.** The Player waveform's tempo sections can now be edited live, while playing:
- drag a change bar, or select a section and use ←/→ (Shift moves by a beat);
- click a BPM label (or press Enter) to type that section's BPM;
- **Add** puts a change at the playhead, **Remove** joins a section to the previous one, **×2/÷2** fix half or double tempo.

**How edits behave.**
- Every edit reaches the metronome, count-in and loops from the next beat.
- Playback never stops, and `player.json` is saved 0.5 s later.
- With the waveform focused, Space, b, e and L keep their Player meaning.

**Rules (your answers Q1–Q5).**
- Detected changes keep their 4-beat limit, widened when the left section's BPM changes.
- Changes added in the Player start on a beat (`player.json` schema 3).
- Every section stays at least 1 beat long.
- ×2/÷2 are disabled outside 20–300 BPM.
- The first section has no Remove.

## Acceptance criteria

| # | Criterion | Tests | Result |
|---|---|---|---|
| TS1 | A change bar drags in 1/64 steps of the left section's BPM and stops 4 beats from its detected position | `ts1_*`, `qa_q2_*`, waveform-geometry, WaveformView.test, GUI `player_tempo_sections` steps 2–3 | PASS |
| TS2 | Typing a BPM on a label changes only that section, and the metronome follows | `ts2_*`, `qa_q5_*`, `qa_scale_*`, player-state.test, GUI step 4 | PASS |
| TS3 | Add splits at the nearest 1/64 step, and the new section has the typed BPM | `ts3_*`, `qa_add_*`, `qa_q3_*`, ControlBar.test, GUI step 6 | PASS |
| TS4 | Remove joins the previous section | `ts4_*`, `qa_first_section_*`, `qa_stale_at_*`, GUI step 7 | PASS |
| TS5 | 93 BPM doubled shows 186 and the metronome ticks at 186 | `ts5_*`, bpm-format.test, `qa_count_in_*`, GUI step 5 (recorded audio) | PASS |
| TS6 | An edit while playing keeps playback going, and the metronome follows from the next beat | `ts6_*` (all 5 kinds of edit), `qa_loops_follow_*`, `qa_count_in_of_a_loop_*` | PASS |
| TS7 | Reopening restores the edited sections from `player.json` | `ts7_*` (exact start positions), `qa_edits_survive_*`, `qa_schema_2_file_*`, GUI step 8 | PASS |
| TS8 | Keys on the focused waveform, while Space/b/e/L keep their Player meaning | WaveformView.test, acceptance_player.test | PASS |
| TS9 | Schema 1/2 files load and are saved as 3; a schema 4 file is read-only | player_state, `qa_schema_1_*`, `qa_newer_schema_*`, `qa_on_beat_*` | PASS |
| TS10 | The Editor shows Player edits and keeps its rules | `qa_editor_sees_player_edits_and_keeps_its_rules`, editor suites | PASS |
| — | An edit is accepted for any start position, including non-dyadic ones such as 93 BPM | `section_guard_accepts_starts_that_went_through_json`, `qa_json_roundtrip_starts_are_accepted_by_every_section_action` | PASS |

**Suites.** `npm test` passes: 578 vitest tests, the whole Rust workspace and clippy. The full GUI suite (`npm run test:gui`) passes after fix round 1.

## Task log

| Step | By | Result |
|---|---|---|
| plan | architect | 11 tasks; you confirmed Q1–Q4 as recommended and accepted Q5 |
| task 1: tempo map, `on_beat` and section operations | implementer | done |
| task 2: `player.json` schema 3 | implementer | done |
| task 3: Player session `edit_sections`, IPC actions | implementer | done |
| task 4: backend acceptance tests ts1–ts7 | implementer | done |
| task 5: TS IPC types | **local (gemma4:26b-a4b-it-qat)** | done; edits exact, but it hit 40 turns and dropped a final newline (I restored it) |
| task 6: `scaleBpm` | **local (gemma4:26b-a4b-it-qat)** | done; correct on the first try (I trimmed an extra trailing blank line) |
| task 7: waveform geometry for free moves | implementer | done |
| task 8: Player frontend state for sections | implementer | done |
| task 9: editable waveform, BPM overlay | implementer | done |
| task 10: Sections group, pane wiring, keys | implementer | done |
| task 11: GUI e2e test, visual review, docs | implementer | done |
| tester | tester | all criteria PASS; 21 extra tests; QD1 (low) found |
| reviewer | reviewer | CHANGES REQUIRED: M1 major, m1 and m2 minor |
| fix round 1 | implementer | M1, m1, m2, QD1 and a nit fixed |
| re-verify | tester, reviewer | PASS, no new defects; APPROVED |

**The local model.** It did both of its tasks correctly, unlike the earlier models. On task 5 it ran out of turns; telling it to run the check once and stop fixed that for task 6.

## Review findings

**Fixed in fix round 1:**
- **M1 (major).** Editing failed for about 8 % of section start positions, for example a change at 93 BPM. The stale-click guard compared the section's start exactly, and serde_json's default parser can be 1 ulp off. Fixed with:
  - the `float_roundtrip` feature;
  - a 1e-6 ms tolerance in the guard;
  - tests that send the start through JSON and back.
- **m1 (minor).** A queued move could land on the wrong section after an Add or Remove. A queued move now keeps its `at`, so the backend refuses it as stale instead.
- **m2 (minor).** The BPM error stayed visible after cancelling the input. Cancel now clears it.
- **QD1 (low).** `add_change` accepted a NaN position. It is refused now.

**Left as is (non-blocking reviewer notes):**
- A same-section follow-up move is dropped silently when the move before it fails.
- A narrow window exists where a queued same-section move reads its index after another section was removed.
- `drainMoves` has some TypeScript casts.

## Deviations and follow-ups

**Deviations from the plan:**
- The backend refuses a move beyond the limit instead of clamping it; the frontend clamps while dragging.
- Two examples in the plan's test lists were wrong ("39.9 ÷2 refused", "186 → ×2 enabled") and are corrected.

**Layout and test notes:**
- **Layout.** The new Sections group re-wraps the control bar. At 1280×800 with the tree folded, the Player pane now scrolls by about 14 px. Playback and Loops sit lower, as the control bar did before at smaller sizes. The Player GUI tests' click positions were updated for this.
- **Flaky test.** `acceptance_stem_extraction_gui::qa_malicious_server_stem_names_are_refused_in_the_ui` timed out once under the parallel GUI run. It passed twice on its own and in the full run after the fix. It is unrelated to this feature.
- **Error wording.** A BPM outside 20–300 sent to the backend gets the generic message "BPM must be 0 or between 20 and 300 with one decimal". The frontend checks the value first, so users see "Enter a BPM between 20 and 300".

**Possible follow-ups:**
- Undo for section edits.
- A clearer selected-section marker; the 4 px bar is visible but subtle.
- Truncating very long backend errors in the Sections message line.

## How to try it

From `src/calliope-gui/`:
- `npm run dev:app`, or `npm run build:app` and install as before.
- Then open a track with detected sections (e.g. Deja-Vu) in the Player.
- Tests: `npm test`, then `npm run test:gui`; `CALLIOPE_GUI_SHOTS=1 npm run test:gui -- visual_review` takes the screenshots.

## Deployment

Laptop only. Nothing changes on archserver.

**Note:** this build writes `player.json` as schema 3. An older build opens those files read-only: it plays with defaults and never overwrites them. Don't mix builds on the same repository.

## Manual checks (laptop with real audio)

- [ ] **M1, Deja-Vu intro.**
  - Metronome On, Play from the start.
  - Select a wrong intro section and Remove it (or press Delete) until the intro is one section.
  - Click its label and type the right BPM: the clicks follow from the next beat, and playback doesn't stop.
- [ ] **M2, half tempo.** Select a 93 BPM section and press ×2. The label shows 186 and the clicks double from the next beat; ÷2 restores it.
- [ ] **M3, by ear.**
  - While playing, focus the waveform, choose a change with ↑/↓ and nudge it with ←/→ until the first click lands on the beat. Shift+←/→ moves by a beat.
  - A detected change stops 4 beats from where it was detected.
  - Dragging a bar shows the offset.
- [ ] **M4, Add.**
  - Pause on the beat where the tempo changes, press Add, type the BPM and press Enter. The bar sits on the playhead, and the first click after it is on the bar.
  - Escape leaves nothing behind.
- [ ] **M5, loops and count-in.** Loop Play with Count-In over an edited section: the count-in uses the edited BPM, and the loop covers the same music.
- [ ] **M6, persistence.**
  - Quit and restart: the sections are as you left them.
  - The Editor shows the same sections: Player-added changes are dimmed there, and detected ones can still be dragged.
- [ ] **M7, keys.** With the waveform focused, Space plays and pauses, and `b`/`e` mark the loop.

Once you've verified it, tick `gui-player-tempo-sections.md` in the roadmap of `specs/overview.md`.
