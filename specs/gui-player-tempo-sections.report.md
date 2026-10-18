# Report: Player tempo sections editing

**Status: DONE.** All three phases are built: phase 1 (requirements 1–7), phase 2 (8–11, added in `22bc967`) and phase 3 (12–16, added in `ce958d3`). Every criterion passes and the reviewer approved each phase. Phase 2's open layout issue (36 px of scroll) is gone with the phase 3 layout: 0 px at 1280×800 with the tree folded.

- Branch `spec/gui-player-tempo-sections`, base `0bd82c3`; phase 2 starts at `3529ee8`, phase 3 at `886cc73`. Not pushed or merged.
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

## Phase 2: section metronome, count-in, names, Start / End

**What you get.** The Sections group now holds:
- the master **Metronome** controls (On/Off and Count In, as before);
- a **name box** next to "Section N of M · BPM";
- **This section: Click** turns the metronome on or off for that section only;
- **This section: Count In** gives a count-in into that section while playing through its start;
- **Start / End** (also Home / End) jump to the section's first beat or its last bar.

Names and toggles are saved in `player.json`, which is now schema 4.

**Rules (your answers to the phase-2 questions).**
- **Count-in:** three clicks at the new section's tempo, the last one a beat before the section. They replace the previous section's clicks in that span.
  - They also play when the previous section's Click is off.
  - When the previous section is shorter than the count-in, they reach back into earlier sections.
  - The section's own first beat clicks only if its Click is on.
- **When it plays:** only when playing through the section start, not when Play or a loop starts there. It is skipped at the end of a repeating loop that ends at that section.
- **Master switch:** Metronome On/Off silenced everything, count-ins included (removed in phase 3).
- **What edits do with the settings:**
  - Add copies Click and gives no name or count-in.
  - Remove drops the removed section's settings.
  - Move, BPM and ×2/÷2 keep them.
  - Editor Autodetect replaces them. The Editor is unchanged.

### Acceptance criteria (phase 2)

| # | Criterion | Tests | Result |
|---|---|---|---|
| TS11 | Count-in into a section: the previous section's clicks make room for it (the new spec criterion) | `ts11_*`, `metronome::clicks_ts11_count_in`, `qa_p2_count_in_into_a_section_whose_own_metronome_is_off_*`, `qa_p2_overlapping_count_ins_*`, GUI `player_section_metronome` | PASS |
| TS12 | Per-section Click; toggling while playing is heard from the next beat; master Off silences all | `ts12_*`, `qa_p2_master_off_*`, `qa_p2_typed_change_copies_*`, ControlBar.test, GUI step 4 | PASS |
| TS13 | Count-in with the previous section off or short, at loop ends; Metronome controls in Sections | `ts13_*`, `qa_p2_loop_end_*`, acceptance_player.test | PASS |
| TS14 | Start / End and Home / End while playing, paused and in a pre-roll | `ts14_*`, `qa_p2_jump_*`, acceptance_player.test, GUI | PASS |
| TS15 | Names and flags survive close and reopen; name cleaning; name box | `ts15_*`, `qa_p2_names_*`, `qa_p2_flags_*`, section-meta.test, GUI step 8 | PASS |
| TS16 | Schema 1–3 upgrade to 4; schema 5 read-only | `ts16_*`, `qa_p2_schema_4_read_*`, player_state | PASS |
| TS17 | Add / Remove / Move / BPM / merge carry the settings as agreed | `ts17_*`, `qa_p2_edits_split_join_move_and_merge_*`, tempo_map | PASS |

**Suites.** `npm test` passes: 1060 Rust tests and 594 vitest tests. In the full GUI suite every test passes except the known stem-extraction flake (below), which passed when re-run alone.

### Task log (phase 2)

| Step | By | Result |
|---|---|---|
| plan phase 2 | architect | tasks 12–22; you kept all recommended answers |
| task 12: tempo map section settings, jump positions | implementer | done |
| task 13: `player.json` schema 4 | implementer | done |
| task 14: schema numbers in other tests | **local (gemma4)** | 11 of 14 edits correct, then out of turns; I did the last 3 |
| task 15: click list | implementer | done |
| task 16: Player session, IPC actions | implementer | done |
| task 17: backend acceptance TS11–TS17 | implementer | done, no code fix needed |
| task 18: TS IPC types | **local (gemma4)** | edits correct, but it committed by itself and reused a test title; I fixed the title and redid the commit |
| task 19: `section-meta.ts` | **local (gemma4)** | content exact; trailing newlines fixed |
| task 20: Player screen state | implementer | done |
| task 21: Sections group UI, Home / End | implementer | done |
| task 22: GUI test, click positions, layout, docs | implementer | done; layout target missed (36 px) |
| tester | tester | all PASS, 15 extra `qa_p2_*` tests, no defects |
| reviewer | reviewer | APPROVED; 2 minors, 3 nits |

**The local model.** It did 3 tasks. All needed small fixes from me, and it once ignored "do not commit". My notes for future builds now tell it never to commit, and I check `git log` after each run.

### Review notes (non-blocking, not fixed)
- **Minor: merge loses settings.** Typing a section's BPM equal to its left neighbour's merges the two, and the right section's name and toggles are lost, the same as Remove. Keeping them would need a rule from you.
- **Minor: click list cost.** `section_clicks` builds every beat to the track end for each count-in section to find one beat. This runs off the audio thread, so playback is unaffected. It could cause a short hitch on very long tracks with many count-ins.
- **Nits:**
  - the per-beat Click lookup has no float tolerance, unlike the count-in code;
  - JS and Rust trim slightly different invisible characters in names;
  - the GUI tests assert on log wording.

### Open issue: layout (resolved in phase 3)
The Sections group is now three rows: the title and name box, then Add … End, then the Metronome and This-section toggles.
- **Measured:** at 1280×800 with the tree folded, the Player pane scrolls by 36 px. That puts the bottom row of the Loops group (Play / Loop On / Count-In) just below the fold.
- **Already done:** the implementer moved the message line into the group's padding (54 → 36 px).
- **Your options:**
  - accept 36 px;
  - make the section toggles and Start / End buttons smaller than 3.25 rem;
  - drop the reserved message strip;
  - tighten the Playback or Loops groups.

Screenshots: `target/gui-shots/player-*-section-selected.png` and `player-*-scrolled.png` (`CALLIOPE_GUI_SHOTS=1 npm run test:gui -- visual_review`).

## Phase 3: two-row layout, no global metronome, default loop end

**What you get.**
- **Layout (your two-row choice and the compact redesign):**
  - Row 1 holds **Sections** and **Loops** side by side.
    - Sections: the title, the BPM field with V, and the name box; then +, −, ×2, ÷2, |◀, ▶|; then Click, Count In and the status line.
    - Loops: three short lines, with the "Loops" title on the third.
  - Row 2 is **Playback**, full width.
  - The disabled BPM −/+/Reset placeholders are hidden until that feature exists.
- **No global metronome:** the global On/Off and Count In are gone, along with their backend command and permission. Each section's Click and Count In decide everything.
- **Play count-in:** Play counts in when the section under the playhead has Count In on, at that section's tempo. Section 1 can have Count In too. Loop Play counts in only with the loop's own Count-In.
- **Loops "+":** a new loop ends where the next tempo section starts, or at the track end. It also ends at the track end on a track without a tempo.
- **Saving:** `player.json` is now schema 5. Older files are converted:
  - global metronome Off (or never set) → every section's Click and Count In off;
  - On → the section settings are kept;
  - global Count In → section 1's Count In.
- **Re-detection** in the Editor keeps section 1's Click (for all new sections) and Count In.

**Measured layout** (from the app's `player layout` log line):

| Window | Tree | Scroll | Loops beside Sections |
|---|---|---|---|
| 1280×800 | folded | 0 px | yes |
| 1280×800 | shown | 122 px | no, below |
| 1024×640 | folded | 237 px | no, below |
| 1024×640 | shown | 650 px | no, below; rows wrap inside the panes, nothing is cut off |

### Acceptance criteria (phase 3)

| # | Criterion | Tests | Result |
|---|---|---|---|
| TS18 (spec: layout) | Under the waveform: Sections (with BPM) and Loops in one row, Playback at the bottom; nothing cut off sideways | acceptance_player.test (pane order), GUI `player_phase3` (`loops_beside=1`, overflow ≤ 14), `qa_p3_layout_never_clips_sideways` (4 sizes) | PASS |
| TS19 (spec: Play count-in) | Play counts in at the tempo of the section under the playhead with Count In, starts at once without | `ts19_*`, `qa_p3_play_count_in_*` (boundary, section 1, track end, no tempo, current loop), `qa_p3_start_jump_*`, `qa_qd3_*`, GUI `player_phase3` | PASS |
| TS20 (spec: no global metronome) | No global On/Off or Count In; clicks only where Click is on | `ts20_*`, `qa_p3_the_master_metronome_command_and_its_permission_are_gone_everywhere`, vitest `qa_p3: no global Metronome…`, `qa_p3: …no command with "metronome"…` | PASS |
| TS21 (spec: loop end) | "+" ends at the next section, or at the track end in the last section | `ts21_*`, `qa_p3_plus_*` (past the audio end, last section, three sections, timestamp mode, 99-loop limit), GUI `loop_add_ends_at_the_next_section…` | PASS |
| TS22 | Schema 1–4 files are converted; schema 6+ read-only | `ts22_*`, `qa_p3_migration_*`, `qa_p3_newer_schemas_stay_read_only_*` | PASS |
| TS23 | Loop Play counts in only with the loop's own Count-In | `ts23_*`, `qa_p3_play_with_a_current_loop_*` | PASS |
| TS24 | Re-detection keeps section 1's Click and Count In | `ts24_*`, `qa_p3_autodetect_*`, beat_detect tests | PASS |

**Suites.**
- `npm test` passes: 599 frontend tests and every Rust test.
- The full GUI suite passes except `acceptance_stem_extraction_gui::qa_malicious_server_stem_names_are_refused_in_the_ui` (see "Flaky test" below).
- One full GUI run had many timeouts in every area, including Library and Import. It ran while the tester and I were compiling at the same time. A re-run with nothing else running passed.

### Task log (phase 3)

| Step | By | Result |
|---|---|---|
| plan phase 3 | architect | tasks 23–31; you changed Q1 (Play mid-section counts in from that section) and the layout to two rows |
| task 23: tempo map: section 1 Count In, `count_in_at`, next section, settings carry | implementer | done |
| task 24: remove the `player_metronome` command | **local (gemma4) failed** → implementer | gemma4 overwrote `build.rs` with one line; reverted, the implementer did the 4 deletions |
| task 25: backend: schema 5, conversion, Play count-in, loop end, re-detection | implementer | done |
| task 26: schema numbers in other tests | **local (gemma4)** | all 7 edits correct |
| task 27: backend acceptance TS19–TS24 | implementer | done, no code fix needed |
| task 28: frontend without the global metronome | implementer | done (ran alongside task 27) |
| task 29: two-row layout | implementer | the pane is only 965 px wide at 1280×800, so Loops wrapped; you chose the compact redesign |
| task 29b: compact layout | implementer | side by side at 1280×800, 0 px scroll |
| task 30: GUI tests, screenshots, `docs/ui.md` | implementer | done; found sideways clipping at 1024×640 |
| task 30b: wrap rows in narrow panes | implementer | nothing clipped |
| task 31: README | **local (gemma4)** | 3½ of 4 replacements; I removed one stale phrase |
| tester | tester | all PASS except QD3 (below); 20 `qa_p3_*` tests |
| reviewer | reviewer | APPROVED; 5 stale doc lines, 3 nits |
| fix round 1 | implementer | QD3 fixed, docs corrected |
| re-verify | tester, reviewer | QD3 PASS with 6 more edge tests; APPROVED |

**The local model (phase 3).**
- Task 26 was perfect.
- Task 31 needed one fix.
- Task 24 was destructive: asked to delete one line, it replaced the whole `build.rs`.

My notes now say to check the diff size first and to avoid giving it deletions in long files.

### Review and test findings
- **QD3, fixed:** a Start jump (|◀) to a section whose start isn't on a whole audio frame (common for detected changes) landed a frame early, so Play skipped that section's Count In. Start now lands on the first frame at or after the section start; End is unchanged.
- **Docs, fixed:** `docs/architecture.md` still listed the removed command, log line and setting, and an old schema-4 rule; `docs/ui.md` described the old message slot.
- **Left as is:**
  - **Empty tempo map:** an old file with no tempo sections has nothing to convert, so its global metronome setting is dropped. This is noted in `docs/architecture.md`.
  - **Hard-coded GUI click positions:** they are re-measured, and any drift shows up as a test failure.
  - **Layout log:** the layout line is logged after every settled resize.
- **Cosmetic:**
  - At 1024×640 with the tree shown, the loop range slider shrinks to a dot.
  - At 1280×800 the Sections pane has some empty space under its third row.
  - When it wraps below Sections, Loops doesn't stretch to the full width.

### Fix round 2 (from your laptop test)
- **Symptom:** select section 2, play into later sections, pause, press |◀ → it jumped back to section 2.
- **Cause:** the selection in the Player screen stayed set while playback moved on, and the section actions use the selection first. The backend's current section was always right.
- **Fix (`b458859`):** the selection drops when the playhead enters another section (by playback, a seek or a jump). A section you pick while paused, or by clicking or ↑/↓ while playing, holds until then. Start/End, the toggles, the name box and the status line all act on the same section.
- **Tests:** two vitest tests reproduce it; both fail without the fix. The reviewer approved. The full GUI suite passed, including the stem-extraction test that had failed in the earlier full runs.

### Flaky test
`acceptance_stem_extraction_gui::qa_malicious_server_stem_names_are_refused_in_the_ui` has now failed in every full parallel GUI run (4 times), always at about 74 s. It passes every time on its own. It is unrelated to this feature and looks like a timing assumption that breaks under parallel load. It is worth its own fix.

## How to try it

From `src/calliope-gui/`:
- `npm run dev:app`, or `npm run build:app` and install as before.
- Then open a track with detected sections (e.g. Deja-Vu) in the Player.
- Tests: `npm test`, then `npm run test:gui`; `CALLIOPE_GUI_SHOTS=1 npm run test:gui -- visual_review` takes the screenshots.

## Deployment

Laptop only. Nothing changes on archserver.

**Note:** this build writes `player.json` as schema 5. Older builds (phase 1: schema 3, phase 2: schema 4) open those files read-only: they play with defaults and never overwrite them. Don't mix builds on the same repository. Opening an older file converts it once: if the old global metronome was Off, every section's Click and Count In start off.

## Manual checks (laptop with real audio)

- [ ] **M1, Deja-Vu intro.**
  - Click on in the sections, Play from the start.
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

- [ ] **M8, count-in into a section.**
  - On Deja-Vu, turn Count In on for the section after the intro. With Click on, play from a few bars before it.
  - The last bar counts "1, 2, 3" at the new tempo, and the new section starts on the next beat.
  - If it doesn't sound right musically, we revisit the count-in rule.
- [ ] **M9, sections without clicks.**
  - Turn Click off for the intro: it is silent, and clicks start with the next section.
  - With the next section's Count In on, the count-in still plays over the silent intro.
  - (Phase 3: there is no global On/Off any more; see M17.)
- [ ] **M10, practising one section.** Loop exactly one section whose next section has Count In, with Loop On. No count-in at the loop end; without the loop it plays.
- [ ] **M11, Start / End.**
  - While playing, End (or the End key) jumps to the section's last bar without a gap, and Start (Home) jumps back.
  - Paused, they only move the playhead.
- [ ] **M12, names.**
  - Name three sections. While playing, the name box follows the playhead, but not while you type.
  - Quit and restart: the names and toggles are back.
  - The Editor shows no names.
- [ ] **M13, layout.**
  - At your usual window size the Sections group is readable from 1–2 m, and the two Count In buttons are easy to tell apart.
  - (The phase 2 scroll issue is resolved by the phase 3 layout; see M14.)

- [ ] **M14, layout.**
  - At 1280×800 with the tree folded: Sections (with BPM) and Loops side by side, Playback at the bottom, and no scrolling.
  - It is readable from 1–2 m, and Play is easy to find.
- [ ] **M15, your existing tracks.**
  - A track used with an older build: with the old Metronome Off it is now silent; with it On, the sections click as before.
  - If its old Count In was on, Play from the start still counts in.
  - Loop Play counts in only with the loop's own Count-In.
- [ ] **M16, Play count-in.**
  - Turn Count In on for a middle section, pause inside it and press Play: three clicks at its tempo, then the music.
  - Pausing in a section without Count In, Play starts at once.
  - Press |◀ on a detected section with Count In, then Play: it counts in (the QD3 fix).
- [ ] **M17, Click only where wanted.** Click off for the intro and on for the verse: the intro is silent and the verse clicks. There is no global On/Off.
- [ ] **M18, default loop end.**
  - Pause inside a section and press "+": the loop ends exactly at the next section (the waveform mark and bar2 agree). In the last section it ends at the track end.
  - Loop On repeats it, and `e` while playing still shortens it.
- [ ] **M20, section target after playing (fix round 2).** Select section 2, play into section 4 or 5, pause and press |◀: it jumps to the start of the section the playhead is in.
- [ ] **M19, re-detect.** Re-run Autodetect in the Editor on a track whose sections are silent and whose first section has Count In: back in the Player it is still silent, and Play from the start still counts in.

Once you've verified it, tick `gui-player-tempo-sections.md` in the roadmap of `specs/overview.md`.
