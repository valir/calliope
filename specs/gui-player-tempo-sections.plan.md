# Plan: Player tempo sections editing

Revision 1. Base `0bd82c3`, branch `spec/gui-player-tempo-sections`. Builds on:
- `specs/gui-play-backing-track.plan.md`: tempo map 2.3, `player.json` 2.5, engine 2.7, BPM field 2.8, IPC 2.9,
  waveform 2.10;
- `specs/stems-beat-autodetect.plan.md`: detected changes (`detected_ms`, 1/64-beat steps, ±256 steps, stretched
  last beat) 2.3, `TempoStore` and the Player release 2.2, the shared `WaveformView` 2.9-2.10, owner decisions 8.

The design below follows the recommended answers of section 8. Where an answer would change a task, the question
says which.

**Revision 2 (phase 2).** The owner extended the spec in `22bc967` (requirements 8-11 and one criterion). Sections 1-8
and tasks 1-11 are built and unchanged; phase 2 is appended as sections 9-16, with tasks 12-22.

**Revision 3 (phase 3).** The owner added requirements 12-16 and four criteria in `ce958d3` (layout, no master metronome,
Play count-in per section, default loop end). Sections 1-16 and tasks 1-22 are built and unchanged; phase 3 is appended as
sections 17-24, with tasks 23-31.

## 1. Summary

The Player's waveform becomes an editor for the tempo sections. You can:
- drag a change bar, or move it with the arrow keys;
- click a section's BPM label and type a new BPM;
- add a change at the playhead;
- remove a change;
- double or halve a section's BPM.

Each edit goes through the open Player session, like a typed BPM does today. The metronome, the count-in and the loops
use the new map from the next beat without stopping playback, and the map is saved in `player.json`. Changes added in
the Player start their section on a beat, just as detected changes do. That needs one new per-change flag, so
`player.json` becomes schema 3.

## 2. Design

### 2.1 What is reused, what is new

Reused unchanged:
- the Player's live tempo path: `PlayerManager::set_bpm` → `apply_tempo_change` (loops keep their time) →
  `sync_grid` (`Cmd::SetGrid`, the engine re-finds the next beat) → `sync_loop` → `mark_dirty` (500 ms write);
- `TempoStore` and the Player release (only the Editor uses them);
- `bases()`/`stretch()` maths for changes that start on a beat;
- the BPM field and `set_bpm_at` (prior spec req. 14);
- `WaveformView`'s drawing, drag and key code (extended, not rewritten);
- the `player_tempo` command. It only gains action kinds, so there are no new commands and no ACL edits.

| Where | What | Pure? |
|---|---|---|
| `src/calliope-gui/src/tempo_map.rs` | `Segment.on_beat`; `starts_on_beat()`; `move_change`, `set_section_bpm`, `scale_section`, `add_change`, `remove_change`; `set_change_offset` delegates to `move_change` | yes |
| `src/calliope-gui/src/player_state.rs` | schema 3: reads `on_beat`; reads 1-3; 4+ read-only | yes |
| `src/calliope-gui/src/player.rs`, `player_ipc.rs` | `PlayerManager::edit_sections(id, SectionEdit, now)`; five new `TempoAction` kinds | Player internals |
| `src/ui/lib/ipc.ts` | `TempoSegment.on_beat`, the new `TempoAction` kinds | types |
| `src/ui/lib/bpm-format.ts` | `scaleBpm(bpm, factor)` | yes |
| `src/ui/lib/waveform-geometry.ts` | `moveRefMs`, `moveLimit`; `dragSteps` gains a `limit` argument | yes |
| `src/ui/lib/player-state.svelte.ts` | section selection, the BPM editor state, the five actions (moves coalesced), messages, log lines | frontend |
| `src/ui/components/player/WaveformView.svelte` | Player mode: every change movable, controlled selection over all sections, label clicks, more keys, the BPM overlay | component |
| `src/ui/components/player/SectionBpmInput.svelte` (new) | the small BPM input drawn over the waveform | component |
| `src/ui/components/player/SectionsSection.svelte` (new) | the "Sections" group of the control bar | component |
| `src/ui/components/player/PlayerPane.svelte`, `src/ui/views/PlayerView.svelte` | wiring; Space, b, e and L keep working while the waveform has the focus | frontend |

### 2.2 Edits go through the live Player session (no release)

**Path.** Every edit is a `player_tempo` action handled by `PlayerManager::edit_sections`, under the `PlayerHost` lock:
1. clone the session's map and apply the pure `TempoMap` operation (2.4); a refusal returns `Err(message)` and
   changes nothing;
2. `state.apply_tempo_change(new)`: loops are re-snapped and keep their time;
3. `sync_grid()`: the engine gets the new beat list through `Cmd::SetGrid`. Its `beat_dirty` re-finds the next beat
   at the current position, so the metronome follows from the next beat and playback is never paused or moved;
4. `sync_loop()`: the current loop's frames and its count-in plan (the BPM at the loop start) are rebuilt;
5. `mark_dirty(now)`: `player.json` is written 500 ms later (and at once on switch, leave, close and exit);
6. log `calliope: player section <kind> id=… index=… segments=<k> bpm=<..>` and return the snapshot.

The count-in of a main Play is computed at Play time from the current map (`plan_at`), so it needs nothing more.
`speed` is not touched (it is always 1.0 until speed change is built; 8, Q5).

**No release.** `TempoStore::replace` (flush + close) stays the Editor's path only:
- **Editor → Player:** an Editor write (detection or move) still flushes the Player's edits first, then closes the
  session. A detection then replaces the whole map, including the Player's edits, as decided for detection.
- **Player → Editor:** the Editor reads the map through `TempoStore::load` (which flushes) on each view entry, so it
  shows the Player's edits.

**Stale indexes.** Every action that names a section also sends that section's current `start_ms` (`at`). Rust
refuses with "The tempo sections changed; try again" when it doesn't match within 1e-6 ms (far below one fine step).
serde_json's default float parsing can be one ulp off for some shortest-repr values, so calliope-gui enables its
`float_roundtrip` feature and the guard still compares with the tolerance. A quick double Remove therefore never removes the wrong change.

### 2.3 Three kinds of changes; `on_beat` and `player.json` schema 3

| Kind | Stored | Starts on a beat | Moves in the Player | Moves in the Editor |
|---|---|---|---|---|
| first segment (anchor, bar 1) | `{start_ms, bpm}` | is beat 1 | never | never |
| detected | `+ detected_ms` | yes (beat-rounded base, stretched last beat) | 1/64-beat steps of the left tempo, at most 4 beats from `detected_ms` | same (unchanged) |
| placed (new: added in the Player, or a typed change once moved) | `+ "on_beat": true` | yes (same rule) | 1/64-beat steps of the left tempo, counted from the left section's start, no limit | no (drawn dimmed, as typed ones are today) |
| typed (BPM field, unchanged) | `{start_ms, bpm}` | no (tick rule) | yes: the first move turns it into a placed change | no |

**Why a flag.** Without it, an added change would follow the tick rule. Its base would round to the nearest
1/16-beat tick and the new section's beats would continue the old global count, so the first click of the new section
could land up to 15/16 beat after the bar the user placed. "Every edit takes effect for the metronome" would then not
hold where the user put the change.
- `Segment.on_beat: bool` (`#[serde(default, skip_serializing_if = "is_false")]`);
- `Segment::starts_on_beat() = detected_ms.is_some() || on_beat`. It replaces `detected_ms.is_some()` in `bases()`,
  `stretch()` and `set_bpm_at`'s "inside the stretched last beat" rule. Detected maps and typed maps compute exactly
  as before.

**Schema 3.** The schema-2 parser rebuilds every segment from `start_ms`/`bpm`/`detected_ms` and drops other keys. A
schema-2 build would therefore silently turn placed changes into typed ones and re-round the bars after them.
- This build reads 1, 2 and 3 and writes 3.
- It reads `on_beat` only as JSON `true` and drops it on the first segment (as `detected_ms`).
- A schema-4 file is read-only.
- Older builds see a schema-3 file as "written by a newer version" and never overwrite it: the existing mechanism,
  as for schema 2.

### 2.4 The operations (`tempo_map.rs`, pure)

Notation:
- `beat(bpm) = 60000 / bpm`;
- `fine(bpm) = beat / 64`;
- for change `i`: `L = segments[i-1]` (left section), `S = segments[i]`, `N = segments[i+1]` if any;
- `end_ms` = the track duration (`f64::INFINITY` from the Editor).

**The section-length rule** (the existing 1-beat rule, now for every edit): every section lasts at least one beat of
its own tempo:
- `S.start - L.start ≥ beat(L.bpm)`;
- `N.start - S.start ≥ beat(S.bpm)`;
- the last change starts at least `beat(S.bpm)` before `end_ms`.

The first section extends backwards and has no lower limit.

1. **`move_change(index, steps, end_ms) -> Result<(TempoMap, i64 applied), String>`**
   - **Refused:**
     - `index == 0`: "The first tempo section can't be moved" (existing);
     - `index` out of range.
   - **Reference:** `ref = S.detected_ms` for a detected change, else `L.start_ms`. The target is
     `ref + steps * fine(L.bpm)`.
     - So a placed change sits on the left section's 1/64-beat grid, counted from the left section's start (its bar 1
       beat 1 when the left section starts on a beat).
     - A typed change's first move snaps it onto that grid.
   - **Limit for a detected change:**
     - `lim = max(256, |cur|)`, where `cur = round((S.start - ref) / fine(L.bpm))`;
     - `|steps| > lim` → `Err("A tempo change can move at most 256 steps from its detected position")` (a frontend
       bug, as today);
     - the widening only matters after the left BPM was changed (e.g. doubled: the same ms are twice the steps). The
       change is never forced to jump; it can still move back towards `detected_ms`.

     Placed and typed changes have no limit.
   - **Clamped** to the section-length rule. "There is no room to move this tempo change" when nothing fits
     (existing).
   - **The result:** `start_ms = ref + applied * fine`. A typed change gets `on_beat = true`. `detected_ms` is kept.
     Later changes keep their `start_ms`.
   - **`set_change_offset(index, steps)`** (the Editor's) keeps its signature and its "Only detected tempo changes can
     be moved; …" refusal for non-detected changes, then calls `move_change(index, steps, f64::INFINITY)`. The Editor
     gains only the widened limit (Q4).
2. **`set_section_bpm(index, bpm) -> Result<TempoMap, String>`** (spec req. 3):
   - `bpm` must be 20..=300 with one decimal (`validate_bpm`); 0 is refused here, with "Enter a BPM between 20 and
     300".
   - It replaces `segments[index].bpm`. `start_ms`, `detected_ms` and `on_beat` are kept and no other segment
     changes.
   - **Equal neighbours are not merged.** A change that keeps the tempo still restarts the beat there, so merging
     would move the clicks and the user can Remove it instead.
   - **Refused** when section `index` is not the last and `N.start - S.start < beat(bpm)`: "Section <index+1> is too
     short for <bpm> BPM (it needs at least one beat)".
3. **`scale_section(index, factor: Double|Halve)`** (spec req. 6):
   - `bpm' = round(bpm * f * 10) / 10`;
   - outside 20..300: "<bpm'> BPM is over 300" / "<bpm'> BPM is under 20";
   - then `set_section_bpm(index, bpm')`;
   - example: 93 ×2 = 186; 186 ÷2 = 93; 93.5 ÷2 = 46.8 (one decimal, so ÷2 then ×2 may differ by 0.1).
4. **`add_change(position_ms, bpm, end_ms) -> Result<(TempoMap, usize new_index), String>`** (spec req. 4):
   - **Refused:**
     - an empty map: "Enter a BPM first" (existing `NO_TEMPO`);
     - `bpm` as in 2;
     - `position_ms < first.start_ms`: "Add tempo changes after the start of the first tempo";
     - `position_ms ≥ end_ms`: "Add tempo changes inside the track".
   - **Placement:**
     - `k = segment_at(position_ms)` (the section it splits);
     - `start = K.start + round((position_ms - K.start) / fine(K.bpm)) * fine(K.bpm)` (the nearest 1/64 step of that
       section);
     - the new segment `{start, bpm, detected_ms: None, on_beat: true}` is inserted at `k+1`.
   - **The section-length rule** for both new neighbours, else "A tempo change needs at least one beat on each
     side". Same-BPM changes are allowed (they re-align the beat).
5. **`remove_change(index) -> Result<TempoMap, String>`** (spec req. 5):
   - `index == 0`: "The first section has no tempo change to remove".
   - Otherwise the segment is removed: its section joins the previous one at the previous BPM.
   - Detected, placed and typed changes are treated alike; a detected position is lost (a new Autodetect brings it
     back).
   - No merge of equal neighbours, and no length check: a section can only get longer.

All five return a new map and never touch the original on a refusal.

### 2.5 IPC

```ts
interface TempoSegment { start_ms: number; bpm: number; detected_ms?: number | null; on_beat?: boolean }
type TempoAction =
  | { kind: 'set-bpm'; bpm: number } | { kind: 'speed-step'; steps: number } | { kind: 'speed-reset' }   // unchanged
  | { kind: 'section-bpm'; index: number; at: number; bpm: number }
  | { kind: 'scale-section'; index: number; at: number; factor: 'double' | 'halve' }
  | { kind: 'add-change'; position: number; bpm: number }          // ms, the playhead when Add was pressed
  | { kind: 'remove-change'; index: number; at: number }
  | { kind: 'move-change'; index: number; at: number; steps: number };  // absolute, from moveRefMs
```

**Rust:**
- `player_ipc::TempoAction` gains the same variants (serde `tag = "kind"`, kebab-case);
- `do_player_tempo` maps them to `SectionEdit` and calls `edit_sections`;
- `move-change` returns the snapshot, and the frontend reads the applied position from it.

**Log lines:**
- Rust: `calliope: player section <bpm|scale|add|remove|move> id=<id> index=<i> segments=<k> bpm=<a,b,..>`
  (+ ` steps=<s> applied=<s'>` for move).
- Frontend: `calliope-ui: player section <kind> index=<i> …` after each answer, and
  `calliope-ui: player sections segments=<k> bpm=<a,b,..> starts=<ms,ms,..>` on `player active` and after every
  edit. The GUI test reads the latter.

### 2.6 Interaction (owner Q1: recommended answer)

**Selection.** One section is the **target** of the actions:
- the section the user selected (`pl.section`), else the section at the playhead (`transport.tempo_index`);
- a click anywhere on the waveform selects the section under the pointer; ↑/↓ on the focused waveform select the
  previous or next one;
- the target's bar is drawn 4 px wide and its label underlined. Section 1 has no bar.
- The selection is cleared on track change, and when it no longer exists after a snapshot (clamped to the last
  section).
- After Add the new section is selected; after Remove, the section it joined.

**Waveform (Player mode, `freeMove`):**
- **Drag** any change bar (8 px grab zone, as in the Editor). The preview is rounded to whole 1/64 steps of the left
  tempo from `moveRefMs` and clamped like Rust.
  - Readout: "+19/64 beat (+0:00.148) · limit ±4 beats" for a detected change; "+19/64 beat (+0:00.148)" for others.
  - One `move-change` on pointer-up. Escape cancels.
- **Label click** (the "120 BPM" text, hit-tested on the rectangles drawn last) opens the BPM overlay for that section.
- **Keys**, when the waveform has the focus (`tabindex=0`, `role="slider"`):

  | Key | Effect |
  |---|---|
  | ↑ / ↓ | select the previous or next section |
  | ← / → | move the selected change by 1/64 beat |
  | Shift+← / Shift+→ | move it by 1 beat |
  | Enter | edit the BPM |
  | Delete or Backspace | remove the change |
  | `a` | add a change at the playhead |

  - Moves are coalesced: one call in flight, the latest target wins.
  - Space, `b`, `e` and `L` keep their Player meaning while the waveform has the focus, so the user can play and
    adjust by ear without leaving it (`PlayerView`'s key filter lets them through for the waveform).
  - `aria-valuetext`: "Section 2 of 3 from 0:08.500, 93 BPM, detected, +0/64 beat".
- **The BPM overlay** (`SectionBpmInput`): a small text input over the label (or, for Add, at the playhead's x),
  bottom-aligned in the waveform, prefilled with the section's BPM (for Add: the BPM at the playhead) and selected.
  - Enter commits if `parseBpm` gives 20..300; otherwise "Enter a BPM between 20 and 300" appears in the Sections
    message line and the input stays open.
  - Escape or blur cancels, so clicking away never adds a change by accident.
  - Placed with `style:` directives (CSP-safe).

**The "Sections" group** in the control bar (a bordered group after BPM, like the others; the visual review places
it):
- **Status line:** "Section 2 of 3 · 93 BPM" (the target), or "No tempo" at BPM 0.
- **Buttons** (3.25 rem high, as the guide asks):

  | Button | Accessible name | Disabled when |
  |---|---|---|
  | Add | "Add a tempo change at the playhead" | BPM 0 |
  | Remove | "Remove the tempo change" | the target is section 1 |
  | ×2 | "Double the section's BPM" | `scaleBpm` gives null (over 300) |
  | ÷2 | "Halve the section's BPM" | `scaleBpm` gives null (under 20) |

  - Add opens the overlay at the playhead position captured when it was pressed.
  - Each disabled button's `title` says why.
- **A reserved one-line message slot** for refusals (`pl.sectionError`), so nothing moves.

**At BPM 0** the waveform is read-only, as today, and the group is disabled with "Type a BPM first".

**Not chosen:**
- a right-click context menu: hidden, mouse-only, and it would need a new menu component;
- keyboard-only actions: not discoverable.

### 2.7 Frontend modules

| File | Change |
|---|---|
| `src/ui/lib/bpm-format.ts` | `scaleBpm(bpm, factor: 2 \| 0.5): number \| null` |
| `src/ui/lib/waveform-geometry.ts` | `moveRefMs(segs, i)` (= `detected_ms ?? segs[i-1].start_ms`); `moveLimit(segs, i)` (detected: `max(256, |offsetSteps|)`, else `Infinity`); `dragSteps(refMs, pointerMs, leftBpm, minMs, maxMs, limit = MAX_OFFSET_STEPS)` |
| `src/ui/lib/player-state.svelte.ts` | `pl.section`, `pl.bpmEdit: {index: number} \| {addAt: number} \| null`, `pl.sectionError`; `targetSection()`; `selectSection`, `editSectionBpm`, `commitSectionBpm(bpm)`, `cancelSectionBpm`, `startAdd()`, `removeChange()`, `scaleSection(f)`, `moveChange(index, steps)` (coalesced); errors go to `pl.sectionError` (not `actionError`); log lines (2.5) |
| `src/ui/components/player/SectionBpmInput.svelte` (new) | props `xPx`, `initial`, `label`, `oncommit(bpm)`, `oncancel()`, `oninvalid(msg)` |
| `src/ui/components/player/WaveformView.svelte` | new optional props `freeMove`, `selected`, `onselect(i)`, `onlabel(i)`, `onremove(i)`, `onadd()`, `bpmEdit`, `oncommitbpm`, `oncancelbpm`, `oninvalidbpm`. With `freeMove` off, the Editor's behaviour (detected changes only, internal selection) is unchanged |
| `src/ui/components/player/SectionsSection.svelte` (new) | the group of 2.6 |
| `src/ui/components/player/PlayerPane.svelte` | passes `freeMove` and the props; adds `SectionsSection` after `BpmSection` |
| `src/ui/views/PlayerView.svelte` | key filter: an element marked `data-player-keys` (the waveform) lets Space, b, e, l/L through |

## 3. Tasks

Rust tasks end with `cargo clippy --workspace --all-targets -- -D warnings` clean; frontend tasks end with `npm run
check` clean (both in `src/calliope-gui/`).

### Task 1: Tempo map: `on_beat` and the section operations
- **files:** `src/calliope-gui/src/tempo_map.rs`. Compiler-driven `on_beat: false` additions to `Segment { .. }`
  literals in:
  - `src/editor_tests.rs`, `src/player_ipc_tests.rs`, `src/player_state.rs`, `src/player_tests.rs`;
  - `src/tempo_detect.rs`, `src/tempo_detect_tests.rs`, `src/tempo_store.rs`;
  - `tests/acceptance_beat_autodetect.rs`.
- **does:**
  - Everything in 2.3 (field, `starts_on_beat()` in `bases`/`stretch`/`set_bpm_at`) and 2.4 (the five operations,
    `set_change_offset` delegating), with the exact messages.
  - `enum Scale { Double, Halve }`.
  - Remove the `#[allow(dead_code)]` markers that no longer apply.
- **tests** (in `tempo_map.rs`):
  - Every existing `tempo_map` test passes unchanged; typed and detected maps give the same `bases()` as before.
  - **Add:**
    - at 10 010 ms into `[(500,120)]` → start on the 120-BPM 1/64-beat grid from 500 (within 1e-9), nearest to
      10 010, `on_beat`;
    - the new section's first beat from `beats_in` is exactly at its start;
    - `ms_at`/`tick_near` stay strictly monotonic and inverse around it (the property loop of the detected tests);
    - refusals: empty map, before the anchor, past `end_ms`, within one beat of either neighbour, bpm 0 or 301.
  - **Move:**
    - a placed change moves by `steps * fine(left)` from the left start, with no ±256 limit;
    - a typed change becomes `on_beat` on its first move;
    - a detected change keeps ±256;
    - after the left BPM is doubled, a change at +200 (now +400) accepts 399 and 400 and refuses 401;
    - clamping at both neighbours and at `end_ms - beat`;
    - the Editor's `set_change_offset` still refuses typed and placed changes with its message.
  - **Section BPM:**
    - only `segments[index].bpm` changes; `detected_ms`/`on_beat` kept; no merge with an equal neighbour;
    - "too short" refusal for a 1-beat section set slower;
    - one decimal enforced.
  - **Scale:** 93 → 186 → 93; 93.5 ÷2 → 46.8; 160 ×2 refused ("320 BPM is over 300"); 39.8 ÷2 refused ("19.9 BPM is under 20"; 39.9 ÷2 rounds to 20.0 and is allowed).
  - **Remove:** index 0 refused; removing a detected change in the middle makes the next detected change's left
    section the previous one (its stretched beat recomputed, monotonic); removing the last change leaves one
    section.
  - **Serde:** `on_beat: false` is not serialised; `true` round-trips.
- **done when:** `cargo test -p calliope-gui --lib tempo_map` and `cargo test --workspace --no-run` pass; clippy is
  clean.
- **routine:** no

### Task 2: `player.json` schema 3
- **files:** `src/calliope-gui/src/player_state.rs`, `src/calliope-gui/src/tempo_store.rs`,
  `src/calliope-gui/src/beat_detect_tests.rs`, `src/calliope-gui/tests/acceptance_player_backend.rs`,
  `src/calliope-gui/tests/acceptance_beat_autodetect.rs`, `src/calliope-gui/tests/gui_editor_e2e.rs`
- **does:**
  - **The code:**
    - `SCHEMA = 3`;
    - `parse_tempo` reads `on_beat` (only JSON `true`) and clears it on the first segment, next to `detected_ms`;
    - schemas 1, 2 and 3 are read; 4+ is read-only.
  - **Test literals that pin the old numbers:**
    - "written files are schema 2" asserts become 3: `player_state.rs` (`back["schema_version"], 2`),
      `beat_detect_tests.rs:146`, `acceptance_beat_autodetect.rs:460` and `:713`,
      `acceptance_player_backend.rs:1196`, `gui_editor_e2e.rs:361`;
    - "newer schema" fixtures `{"schema_version": 3 …}` become 4: `player_state.rs:523`, `tempo_store.rs:100`,
      `acceptance_beat_autodetect.rs:611`, `acceptance_player_backend.rs:1259`.

    Fixtures that *read* schema 2 stay 2 (they now test reading an older schema). `track.json` and repository
    schema literals are unrelated: leave them.
  - **New tests** in `player_state.rs`:
    - a v2 file with a detected change loads unchanged and is written as 3;
    - `on_beat: true` round-trips;
    - `on_beat` on the first segment is dropped;
    - `"on_beat": "yes"` / `1` is ignored and the segment kept.
- **done when:** `cargo test -p calliope-gui --lib player_state`, `… --lib tempo_store`, `… --lib beat_detect`,
  `cargo test -p calliope-gui --test acceptance_player_backend --test acceptance_beat_autodetect` pass, and
  `cargo test -p calliope-gui --features e2e-hooks --test gui_editor_e2e --no-run` builds.
- **routine:** no

### Task 3: Player session: `edit_sections` and the IPC actions
- **files:** `src/calliope-gui/src/player.rs`, `src/calliope-gui/src/player_tests.rs`,
  `src/calliope-gui/src/player_ipc.rs`, `src/calliope-gui/src/player_ipc_tests.rs`
- **does:**
  - **`player.rs`:** `pub enum SectionEdit { Bpm{index, at, bpm}, Scale{index, at, factor}, Add{position, bpm},
    Remove{index, at}, Move{index, at, steps} }` and `PlayerManager::edit_sections(id, edit, now) ->
    Result<PlayerSnapshot, String>`:
    - the `at` guard (2.2);
    - the operation with `end_ms` = the session's duration;
    - steps 2-6 of 2.2, with `loop_message = None`.
  - **`player_ipc.rs`:** the five `TempoAction` variants (2.5), mapped in `do_player_tempo`.
- **tests:**
  - **`player_tests.rs`** (ManualBackend; extend the existing set-BPM/grid tests' helpers rather than new rigs):
    - each edit sends a new grid (`Cmd::SetGrid` observed through the engine: the next click after the edit is on
      the new map) and marks dirty: on disk after `tick(t0+500ms)`, not at `t0+499`;
    - a stale `at` → exact message, map unchanged, no write;
    - a refusal leaves the map, the loops and the dirty flag untouched;
    - loops keep their ms across an edit.
  - **`player_ipc_tests.rs`:** the JSON of each action deserialises (`{"kind":"move-change","index":1,"at":8500.0,
    "steps":-3}` …); an unknown factor is rejected.
- **done when:** `cargo test -p calliope-gui --lib player` passes; clippy is clean.
- **routine:** no

### Task 4: Backend acceptance tests for the spec's criteria
- **files:** `src/calliope-gui/tests/acceptance_player_backend.rs`
- **does:**
  - **Helper:** a new section "tempo sections (gui-player-tempo-sections)" with `rig_with(title, player_json)`,
    which writes `player.json` into the temp copy before `open`. Reuse `rig`, `clicks`, `onsets` and `expected`.
  - **Fixture map:** Practice Song with `{"schema_version":2, "tempo":{"beats_per_bar":4,"segments":[
    {"start_ms":500,"bpm":120},{"start_ms":8500,"bpm":93,"detected_ms":8500}]}}`.
  - **Tests:**
    - `ts1_drag_moves_in_fine_steps_and_stops_at_4_beats`: `move-change` 1, 19, 256 and 400 →
      `start = 8500 + applied * 7.8125` (19 → 8648.4375, 400 → applied 256 = 10500); `detected_ms` stays 8500.
    - `ts2_section_bpm_changes_only_that_section_and_metronome_follows`:
      - section 0 → 110: section 1 still 93 from 8500;
      - metronome on, play from 2000 ms: click spacing 545.45 ms (±1 frame) up to the stretched beat, then 645.16 ms
        after 8500.
    - `ts3_add_splits_at_nearest_fine_step_with_typed_bpm`: add at 15 010 ms, 140 BPM → start on the 93-BPM fine grid
      from 8500 nearest 15 010, `on_beat`; a click at exactly that start; then clicks 428.57 ms apart.
    - `ts4_remove_joins_previous`: remove 1 → one section at 120 and clicks 500 ms apart past 8500.
    - `ts5_double_93_gives_186_and_ticks_at_186`: `scale-section` 1 double → bpm 186; `transport.bpm` 186 at
      12 000 ms; clicks 322.58 ms apart after 8500.
    - `ts6_edits_while_playing_continue_and_follow_from_next_beat`, for each kind of edit:
      - play from 9000 with metronome on; pull 0.5 s; edit; pull 1.5 s;
      - the transport stays playing and the audio equals the source sample by sample across the edit (no fade, no
        jump);
      - the first click after the edit is the new map's first beat after the edit position (±1 frame), and no click
        sounds between the edit and it other than the old map's beats before the edit.
    - `ts7_edits_survive_close_and_reopen`: all five edits, `close`, reopen → the same segments (including
      `on_beat` and `detected_ms`) from `player.json` (schema 3).
- **done when:** `cargo test -p calliope-gui --test acceptance_player_backend ts` passes in under 20 s.
- **routine:** no

### Task 5: TS IPC types for the section actions
- **files:** `src/calliope-gui/src/ui/lib/ipc.ts`, `src/calliope-gui/src/ui/lib/ipc-player.test.ts`
- **does:** exact edits:
  1. In `ipc.ts`, in `interface TempoSegment`, after the line `  detected_ms?: number | null;` add:
     ```ts
       /** set on changes added or moved in the Player: the section starts on a beat */
       on_beat?: boolean;
     ```
  2. In `ipc.ts` replace
     ```ts
       | { kind: 'speed-reset' };
     ```
     (the last line of `export type TempoAction`) with
     ```ts
       | { kind: 'speed-reset' }
       | { kind: 'section-bpm'; index: number; at: number; bpm: number }
       | { kind: 'scale-section'; index: number; at: number; factor: 'double' | 'halve' }
       | { kind: 'add-change'; position: number; bpm: number }
       | { kind: 'remove-change'; index: number; at: number }
       | { kind: 'move-change'; index: number; at: number; steps: number };
     ```
  3. In `ipc-player.test.ts`, insert this test directly before the file's last line (the `});` that closes
     `describe('player ipc wrappers', …)`):
     ```ts
       it('player_tempo passes the section actions unchanged', async () => {
         const seen: unknown[] = [];
         mockIPC((cmd, args) => {
           seen.push([cmd, args]);
           return null;
         });
         await playerTempo('t1', { kind: 'move-change', index: 1, at: 8500, steps: -3 });
         await playerTempo('t1', { kind: 'scale-section', index: 0, at: 500, factor: 'double' });
         expect(seen).toEqual([
           ['player_tempo', { id: 't1', action: { kind: 'move-change', index: 1, at: 8500, steps: -3 } }],
           ['player_tempo', { id: 't1', action: { kind: 'scale-section', index: 0, at: 500, factor: 'double' } }],
         ]);
       });
     ```
     (`mockIPC`, `playerTempo`, `it` and `expect` are already imported there; nothing else changes.)
- **done when:** `cd src/calliope-gui && npx vitest run src/ui/lib/ipc-player.test.ts && npm run check` passes.
- **routine:** yes

### Task 6: `scaleBpm`
- **files:** `src/calliope-gui/src/ui/lib/bpm-format.ts`, `src/calliope-gui/src/ui/lib/bpm-format.test.ts`
- **does:** exact edits:
  1. Append to `bpm-format.ts`:
     ```ts
     /** Doubles or halves a BPM, rounded to one decimal; null when the result is outside 20..300. Mirrors Rust `scale_section`. */
     export function scaleBpm(bpm: number, factor: 2 | 0.5): number | null {
       const v = Math.round(bpm * factor * 10) / 10;
       return v >= 20 && v <= 300 ? v : null;
     }
     ```
  2. In `bpm-format.test.ts` change the line `import { formatBpm, parseBpm } from './bpm-format';` to
     `import { formatBpm, parseBpm, scaleBpm } from './bpm-format';`, and append at the end of the file:
     ```ts
     describe('scaleBpm', () => {
       it('doubles and halves with one decimal', () => {
         expect(scaleBpm(93, 2)).toBe(186);
         expect(scaleBpm(186, 0.5)).toBe(93);
         expect(scaleBpm(93.5, 0.5)).toBe(46.8);
         expect(scaleBpm(150, 2)).toBe(300);
         expect(scaleBpm(40, 0.5)).toBe(20);
       });
       it('refuses results outside 20..300', () => {
         expect(scaleBpm(160, 2)).toBeNull();
         expect(scaleBpm(39.8, 0.5)).toBeNull();
       });
     });
     ```
- **done when:** `cd src/calliope-gui && npx vitest run src/ui/lib/bpm-format.test.ts && npm run check` passes.
- **routine:** yes

### Task 7: Waveform geometry for free moves
- **files:** `src/calliope-gui/src/ui/lib/waveform-geometry.ts`, `src/calliope-gui/src/ui/lib/waveform-geometry.test.ts`
- **does:**
  - `moveRefMs` and `moveLimit` (2.7);
  - `dragSteps` gains `limit = MAX_OFFSET_STEPS` (Infinity allowed) and keeps its current behaviour by default (the
    Editor calls it without the argument);
  - the end bound is passed as `maxMs` by the caller.
- **tests:**
  - the Task 1 numbers (placed change on the left grid; detected widened to 400; clamps), so frontend and Rust
    agree;
  - the existing `dragSteps` tests unchanged.
- **done when:** `npx vitest run src/ui/lib/waveform-geometry.test.ts` passes.
- **routine:** no

### Task 8: Player frontend state for sections
- **files:** `src/calliope-gui/src/ui/lib/player-state.svelte.ts`, `src/calliope-gui/src/ui/lib/player-state.test.ts`
- **does:**
  - The state and actions of 2.7. Every action sends `at = segments[index].start_ms` from the current snapshot.
  - **`moveChange`:** one call in flight; the latest target is sent next, re-reading `at` from the new snapshot.
  - **`startAdd`:** captures `pl.transport.position_ms` and opens `bpmEdit = {addAt}`.
  - **Selection rules** (2.6).
  - **Errors** go to `pl.sectionError`, and the next successful edit clears it.
  - **Log lines** (2.5).
- **tests** (mockIPC):
  - each action's exact `player_tempo` arguments;
  - the target is the selection, else `tempo_index`;
  - selection cleared on track change and clamped after a shorter snapshot;
  - add selects the new section; remove selects the joined one;
  - move coalescing: three key moves while one call is pending → exactly two calls, the second with the latest
    steps and the new `at`;
  - a refusal sets `sectionError` and changes nothing else;
  - a stale-id answer is dropped (existing generation guard).
- **done when:** `npx vitest run src/ui/lib/player-state.test.ts` passes; `npm run check` is clean.
- **routine:** no

### Task 9: Editable Player waveform and the BPM overlay
- **files:** `src/calliope-gui/src/ui/components/player/WaveformView.svelte`,
  `src/calliope-gui/src/ui/components/player/SectionBpmInput.svelte` (new),
  `src/calliope-gui/src/ui/components/player/WaveformView.test.ts`
- **does:**
  - The Player mode of 2.6/2.7 behind `freeMove`:
    - drag of any change with `moveRefMs`/`moveLimit` and the end bound;
    - controlled selection over all sections;
    - click-to-select, label hit-test (rectangles stored while drawing);
    - the target drawn 4 px with an underlined label;
    - the extra keys, with no `preventDefault` on Space;
    - `data-player-keys` on the canvas;
    - the overlay rendered from `bpmEdit`.
  - `SectionBpmInput` validates with `parseBpm` (0 refused).
  - The Editor mode is untouched, and its tests stay green.
- **tests** (extend `WaveformView.test.ts`; canvas mock as today):
  - Player mode: a drag of a **placed** change by N px → one `onmove(i, steps)` from the left start; a detected
    change still stops at 256; Escape cancels;
  - a click on a label → `onlabel(i)`; a click elsewhere → `onselect(section under pointer)`;
  - keys: ↑/↓ cover section 0 too, ←/→ ±1, Shift ±64, Enter → `onlabel`, Delete/Backspace → `onremove`, `a` →
    `onadd`; Space is not prevented;
  - the overlay: Enter with "186" → `oncommitbpm(186)`; "0" and "301" → `oninvalidbpm('Enter a BPM between 20 and
    300')` and it stays open; Escape and blur → `oncancelbpm`;
  - Editor mode (no `freeMove`): the existing tests unchanged; typed and placed changes don't start a drag.
- **done when:** `npx vitest run src/ui/components/player/WaveformView.test.ts src/ui/components/editor/` passes;
  `npm run check` is clean.
- **routine:** no

### Task 10: Sections group, pane wiring, Player keys
- **files:** `src/calliope-gui/src/ui/components/player/SectionsSection.svelte` (new),
  `src/calliope-gui/src/ui/components/player/PlayerPane.svelte`, `src/calliope-gui/src/ui/views/PlayerView.svelte`,
  `src/calliope-gui/src/ui/components/player/ControlBar.test.ts`, `src/calliope-gui/src/ui/acceptance_player.test.ts`
- **does:**
  - The group of 2.6, wired to player-state.
  - `PlayerPane` passes the Player-mode props to `WaveformView` and adds the group after `BpmSection`.
  - `PlayerView`'s key filter lets Space, b, e and l/L through from `[data-player-keys]`.
- **tests:**
  - **`ControlBar.test.ts`:**
    - the group's buttons, names and disabled reasons (BPM 0; section 1 → Remove disabled; 93 → both enabled, 160
      → ×2 disabled with its title);
    - clicks → the right `player_tempo` actions;
    - the status line;
    - the message slot keeps its height.
  - **`acceptance_player.test.ts`:**
    - Space on the focused waveform toggles play;
    - `←` on the focused waveform moves the change and does **not** select the previous loop;
    - with the focus elsewhere, ←/→ still select loops.
- **done when:** `npx vitest run src/ui/components/player/ src/ui/acceptance_player.test.ts src/ui/views/` passes;
  `npm run check` is clean.
- **routine:** no

### Task 11: GUI end-to-end, visual review, docs
- **files:** `src/calliope-gui/tests/gui_player_e2e.rs`, `docs/ui.md`, `src/calliope-gui/README.md`
- **does:**
  - **New test `player_tempo_sections`** in `gui_player_e2e.rs`. Use the existing helpers; set the capture as in
    `tempo_metronome_count_in`. Before start, write the Task 4 fixture `player.json` into the copy of Practice Song.
    Steps:
    1. pick the track → `player sections segments=2 bpm=120,93`; screenshot `player-sections`;
    2. drag the change bar far right → `player section move index=1 … applied=256`; `player.json` change start
       10 500, `detected_ms` 8500;
    3. focus the waveform, `←` → `applied=255`;
    4. click the "120 BPM" label, type 110, Enter → `player sections … bpm=110,93`; screenshot `player-section-bpm`
       (with the overlay open, taken before Enter);
    5. seek 11.0 s, Metronome On, Play; after about 1 s click ×2 (section 1 is the target through the playhead);
       after about 1.5 s Pause:
       - no `player pause`/`player play` line in between;
       - capture click spacing ≈ 645 ms before and ≈ 322.6 ms after (±5 ms);
       - `player sections … bpm=110,186`;
    6. seek 15.0 s, click Add, type 140, Enter → 3 segments; the new start within half a 186-BPM fine step of
       15 000 ms;
    7. Remove (the new section is selected) → 2 segments;
    8. close and restart the app on the same dirs, pick the track → `player sections segments=2 bpm=110,186` with
       the same starts; `player.json` `schema_version` 3.
  - **`visual_review`** gains the states "section selected" and "overlay open" (both themes; 1280x800 and 1024x640;
    the tree shown and folded). Review:
    - where the Sections group goes, and that nothing moves;
    - label and overlay legibility at 1-2 m;
    - the 4 px selected bar against the loop marks.
  - **Docs:**
    - `docs/ui.md` "Decided by the team": Player sections editable, the Sections group, the keys, the overlay
      rules; replace "In the Player the waveform is read-only" and the Player keys line;
    - README usage.
- **done when:**
  - `cd src/calliope-gui && npm run test:gui -- player_tempo_sections` passes (private display);
  - the screenshots in `target/gui-shots/` were reviewed;
  - `npm test` and `npm run test:gui` pass.
- **routine:** no

**Order:**
- Rust: 1 → 2 → 3 → 4.
- Frontend: 5 and 6 any time; 7 after 1; 5 → 8 → 9 → 10.
- Last: 11.

## 4. Test strategy

- **Pure Rust units:**
  - `tempo_map` operations, refusals, the `on_beat` base and stretch rules, monotonic `ms_at`/`tick_near` (Task 1);
  - schema 3 read/write (Task 2).
- **Player session** (`player_tests`, ManualBackend, injected `Instant`s): grid upload, dirty/500 ms write, the stale
  guard, refusals that change nothing (Task 3).
- **Audio behaviour** (`acceptance_player_backend`, ManualBackend):
  - click onsets at exact frames before and after each edit;
  - the source samples continuous across an edit while playing;
  - reopen from disk (Task 4).

  No real device: `CpalBackend` is never constructed in tests (static test unchanged).
- **Frontend:** vitest + mockIPC for state, geometry, the waveform (canvas mock), the overlay, the group and the keys
  (Tasks 5-10); `svelte-check`.
- **GUI:** one new test in `tests/gui_player_e2e.rs` on a private display with `CALLIOPE_E2E_AUDIO=capture:` click
  analysis, plus screenshots (Task 11).
- **Not involved:** calliope-stems, the network, archserver and the edge-AI server. Detection is not part of this
  feature; Player edits on a detected map use a written fixture `player.json`.
- **Whole suite:** `cd src/calliope-gui && npm test`, then `npm run test:gui`.

## 5. Deployment

Laptop only:
- `cd src/calliope-gui && npm run build:app`, then install the binary as before;
- nothing changes on archserver.

**Note:** `player.json` files written by this build are schema 3. The previous build (schema 2) opens them read-only.
It plays with defaults and never overwrites them, so don't mix builds on the same repository.

## 6. Manual checks (owner: laptop with real audio)

- **M1: Deja-Vu intro fix.**
  1. Open Deja-Vu in the Player (detected sections), Metronome On, Play from the start.
  2. Click a wrong-tempo intro section, then Remove (or select it and press Delete) until the intro is one section.
  3. Click its label and type the right BPM: the clicks follow from the next beat, and playback never stops.
- **M2: Half-time section.** Select a section the detector found at half tempo (e.g. 93), then ×2: the label shows 186
  and the clicks double from the next beat. ÷2 brings it back.
- **M3: By ear.**
  1. While playing, focus the waveform, select a change with ↑/↓ and press ←/→ until the first click of the new
     section lands on the music's beat. Shift+←/→ moves by a beat.
  2. A detected change stops 4 beats from where it was detected.
  3. Drag a bar: the readout shows the offset.
- **M4: Add.**
  1. Pause on a beat where the tempo changes, press Add, type the BPM and press Enter: the new bar sits on the
     playhead.
  2. The first click after it is on the bar.
  3. Escape leaves nothing behind.
- **M5: Loops and count-in.**
  1. With a loop inside an edited section, Loop Play with Count-In: the count-in uses the edited BPM.
  2. The loop still covers the same music.
- **M6: Persistence.** Quit and restart: the sections are as you left them. In the Editor the same sections show; the
  changes added in the Player are dimmed there, and detected ones can still be dragged.
- **M7: Keys.** With the waveform focused, Space still plays and pauses, and `b`/`e` still mark the loop.

## 7. Acceptance mapping

| # | Criterion | Spec | Tasks | Tests |
|---|---|---|---|---|
| TS1 | Given a track with detected sections, when the user drags a change bar in the Player, then it moves in 1/64 steps of the left section's BPM and stops at 4 beats from its detected position | req 1, 2 | 1, 3, 7, 9, 11 | tempo_map, `ts1_*`, waveform-geometry, WaveformView.test, gui `player_tempo_sections` (steps 2-3) |
| TS2 | Given a section, when the user types a new BPM on its label, then only that section changes and the metronome follows it | req 3 | 1, 3, 8, 9, 11 | tempo_map, `ts2_*`, player-state.test, WaveformView.test (overlay), gui (step 4) |
| TS3 | Given a section, when the user adds a change inside it, then the section is split at the nearest 1/64 step and the new section has the typed BPM | req 4 | 1, 3, 8, 10, 11 | tempo_map, `ts3_*`, ControlBar.test, gui (step 6) |
| TS4 | Given a change, when the user removes it, then its section joins the previous one | req 5 | 1, 3, 8, 10, 11 | tempo_map, `ts4_*`, ControlBar.test, gui (step 7) |
| TS5 | Given a section at 93 BPM, when the user doubles it, then it shows 186 and the metronome ticks at 186 there | req 6 | 1, 3, 6, 10, 11 | tempo_map, `ts5_*`, bpm-format.test, ControlBar.test, gui (step 5, capture) |
| TS6 | Given any of these edits while playing, then playback continues and the metronome follows the new map from the next beat | req 7 | 3, 4, 11 | player_tests, `ts6_*`, gui (step 5) |
| TS7 | Given any of these edits, when the Player is closed and reopened, then the edited sections are restored from player.json | req 7 | 2, 3, 4, 11 | player_state, `ts7_*`, gui (step 8) |
| TS8 (added) | Given the waveform has the focus, then ↑/↓ select a section, ←/→ and Shift+←/→ move the selected change, Enter edits its BPM, Delete removes it and `a` adds one at the playhead, while Space, b, e and L keep their Player meaning | req 1-6 (keyboard) | 9, 10 | WaveformView.test, acceptance_player.test |
| TS9 (added) | Given a `player.json` of schema 1 or 2, then it loads with the same map and loops; files are written as schema 3; a schema-4 file is never overwritten | 2.3 | 2 | player_state, acceptance_player_backend, acceptance_beat_autodetect |
| TS10 (added) | Given Player edits, when the user opens the Editor, then it shows the same sections; detected changes still move there with the same limit; changes added in the Player are shown but not movable | 2.2, Q4 | 1, 2 | tempo_map (`set_change_offset`), existing editor_ipc_tests / gui_editor_e2e kept green |

## 8. Owner decisions (2026-10-14: Q1-Q4 the recommended answers, Q5 accepted as listed)

1. **How are Add, Remove, ×2 and ÷2 reached?** (the spec's question)
   - (a) a "Sections" group in the control bar acting on the selected section (else the one at the playhead), plus
     keys on the focused waveform (Enter, Delete, `a`, arrows) and label clicks;
   - (b) a right-click context menu on the waveform;
   - (c) keys only.

   **Recommended: (a).** Big buttons are visible and usable while playing, and a menu is hidden and mouse-only.
   The BPM field keeps today's behaviour: typed away from a change, it still inserts a change (prior spec req.
   14). If you'd rather the BPM field only set the section at the playhead, say so: that is a small change in
   `set_bpm_at` plus updates to the gui-play-backing-track tests.
2. **Does an edited detected change keep its detected position and the 4-beat limit?** (the spec's question)
   - (a) yes, through BPM edits and moves; if the left section's BPM changes, the limit is widened just enough that
     the change never has to jump;
   - (b) it becomes a free change after its first Player edit;
   - (c) a separate "Release" action.

   **Recommended: (a)**, as req. 2 says. To put a change further away, Remove it and Add a new one, which is free.
3. **Changes added in the Player start their section on a beat (`player.json` schema 3).**
   - (a) yes: the last beat before the change is stretched, as for detected changes, so the first click of the new
     section is exactly on the bar you placed. That needs a new per-change flag, so `player.json` becomes schema 3,
     and the previous build opens such files read-only. A typed change (BPM field) becomes such a change when you
     move it.
   - (b) no: they follow the typed-BPM tick rule. There is no schema change, but the first click after an added
     change can land up to 15/16 of a beat after the bar.

   **Recommended: (a).** With (b), Tasks 1-2 shrink (no flag, no schema bump), but TS6's "follows from the next beat"
   only holds approximately at added changes.
4. **The Editor** ("Changes to the Editor view need confirmation first").
   - With no Editor edits, the Editor shows Player-added changes dimmed and not movable, like typed ones today. Its
     tooltip keeps saying "This tempo change was typed in the Player; only detected changes can be moved".
   - One unavoidable side effect comes from the shared Rust rule: after a left BPM change, a detected change's limit
     widens (Q2) in the Editor too.

   **Recommended:** confirm the widened limit for both views, and allow one Editor text change: the tooltip becomes
   "This tempo change was set in the Player; move it there". Nothing else in the Editor changes. If you decline, the
   tooltip stays as it is.
5. **Smaller rules the plan applies unless you say otherwise:**
   - a section is at least **1 beat** of its own tempo after any edit (the existing rule; the 1-bar minimum stays a
     detection rule only), and edits that break it are refused with a message;
   - ×2/÷2 are disabled when the result is outside 20..300 BPM, and results are rounded to 0.1 BPM;
   - equal neighbouring BPMs are **not** merged after a section edit (a same-tempo change still re-aligns the beat);
   - the first section has no Remove (no change to remove);
   - removing a detected change loses its detected position (Autodetect again restores it);
   - section edits don't touch the speed. `-`/`+`/`Reset` are still disabled (speed comes later); when speed exists,
     the labels show the section's own BPM, not the speed-scaled one.

---

# Phase 2: per-section metronome, count-in, names and jumps

Spec requirements 8-11 and the new criterion (commit `22bc967`). Base: the finished phase 1 (tasks 1-11, fix round 1).
The design follows the recommended answers of section 16; where an answer would change a task, the question says which.

## 9. Summary (phase 2)

Each tempo section gets three settings: a **name**, **metronome on/off** and **count-in**. They are stored in the
section's segment in `player.json`, which becomes schema 4.
- The metronome clicks only in sections whose metronome is on. The existing Metronome On/Off stays as the master switch.
- A section with count-in gets three clicks at its own tempo just before it, replacing the previous section's last
  clicks, so you hear the new tempo before it starts.
- The Metronome buttons move into the Sections group. The group also gets the name box next to "Section 6 of 8 · 93
  BPM", the two section toggles, and Start / End buttons that jump the playhead within the section.

## 10. Design (phase 2)

### 10.1 What is reused, what is new

Reused unchanged:
- the engine (`Engine`, `Cmd`, `render`): it already plays any sorted list of click frames (`Cmd::SetGrid`) when its
  metronome flag is on (`Cmd::SetMetronome`). Phase 2 only changes **which** frames the manager sends;
- the live edit path of 2.2 (`edit_sections` → `apply_tempo_change` → `sync_grid` → `sync_loop` → `mark_dirty`) and its
  stale `at` guard;
- the Play and loop count-in (the pre-roll before the audio starts, `plan_at`, `Count In` and the loop's `Count-In`);
- loop Prev/Next's instant jump (`Cmd::Jump`), for Start / End;
- `player_tempo` (only new action kinds: no new commands, no ACL edits), the selection / target rule of 2.6.

| Where | What | Pure? |
|---|---|---|
| `src/calliope-gui/src/tempo_map.rs` | `SectionMeta {name, metronome, count_in}` flattened into `Segment`; carry rules in `add_change`, `remove_change`, `set_bpm_at`; `set_section_flags`, `set_section_name`, `section_jump_ms` | yes |
| `src/calliope-gui/src/player_state.rs` | schema 4: reads the three keys; reads 1-4; 5+ read-only | yes |
| `src/calliope-gui/src/metronome.rs` | `section_clicks(map, dur_ms, wrap)`: the click list with section flags and count-ins | yes |
| `src/calliope-gui/src/player.rs`, `player_ipc.rs` | the grid comes from `section_clicks`; it is re-sent when the repeating loop changes; `SectionEdit::Flags`, `SectionEdit::Name`; `jump_section`; three new `TempoAction` kinds | Player internals |
| `src/ui/lib/ipc.ts` | `TempoSegment.name/metronome/count_in`, the new `TempoAction` kinds | types |
| `src/ui/lib/section-meta.ts` (new) | `sectionClicks`, `sectionCountIn`, `cleanSectionName`, `MAX_SECTION_NAME` | yes |
| `src/ui/lib/player-state.svelte.ts` | `setSectionFlags`, the name edit (`pl.nameEdit`), `jumpSection`, log lines | frontend |
| `src/ui/components/player/SectionsSection.svelte` | name box, the Metronome sub-group (moved in), the "This section" toggles, Start / End | component |
| `src/ui/components/player/MetronomeSection.svelte` | deleted (its two buttons move into the Sections group) | component |
| `src/ui/components/player/PlayerPane.svelte`, `src/ui/views/PlayerView.svelte` | no Metronome group; Home / End keys | frontend |

### 10.2 Data model and `player.json` schema 4

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionMeta {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,                // "" = no name; trimmed, no control characters, at most 40 characters
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub metronome: bool,             // default true: older maps click everywhere, as today
    #[serde(default, skip_serializing_if = "is_false")]
    pub count_in: bool,              // never on the first segment
}
// impl Default for SectionMeta: name "", metronome true, count_in false
pub struct Segment { start_ms, bpm, detected_ms, on_beat, #[serde(flatten)] pub meta: SectionMeta }
```

- **On disk**: `{"start_ms": 8500, "bpm": 93, "detected_ms": 8500, "name": "Verse", "metronome": false, "count_in": true}`.
  Defaults are not written, so a map with no names and default flags is written exactly as in schema 3.
- **In the snapshot** the same keys appear (missing = default); the frontend reads them only through `section-meta.ts`.
- The top-level `"metronome": {"on", "count_in"}` stays: `on` is the master switch, `count_in` the Play count-in.
- **Why one flattened struct:** the three settings travel with their segment through every operation, and the 32
  existing `Segment { .. }` literals gain one field (`meta: SectionMeta::default()`), not three.
- **Why schema 4:** the schema-3 parser rebuilds each segment from the four known keys and drops the rest. A schema-3
  build would open a phase-2 file, lose the names and flags, and write it back. With 4, the existing mechanism applies:
  older builds see "written by a newer version" and never overwrite it.
- **Upgrade of v1-v3:** read as before. Every section gets metronome on, no count-in and no name, so the clicks are
  exactly as before. The file is written as 4 on the next save.
- **Lenient read** (`parse_tempo`):
  - `name` only if it is a JSON string: control characters removed, trimmed, cut to 40 characters;
  - `metronome` is false only for JSON `false`;
  - `count_in` is true only for JSON `true`, and is cleared on the first segment (like `detected_ms` and `on_beat`).

### 10.3 Section settings through the edits

| Edit | Name, metronome, count-in |
|---|---|
| move a change (Player or Editor), section BPM, ×2 / ÷2 | kept: the segment is the same |
| Add (splits section k) | section k keeps its own; the new section gets **section k's metronome flag**, no name and no count-in (so the clicks you hear don't change until you change them) |
| Remove change i (section i joins i-1) | section i-1 keeps its own; section i's name and flags are lost |
| BPM field: a new typed change | as Add (copies the metronome flag of the section it splits) |
| BPM field: equal neighbours merged, or BPM 0 removing a segment | as Remove; when the first segment goes, the new first one loses its count-in |
| Editor Autodetect | the whole map is replaced (decided for detection), so names and flags are lost (Q8) |

A count-in belongs to the change at the start of its section, so it moves with that change. When the section before it
changes (after a Remove), the count-in now replaces that section's last clicks instead.

### 10.4 The click list (`metronome::section_clicks`, pure)

The engine's grid becomes the list of clicks to sound, computed off the audio thread by the manager:

```text
section_clicks(map, dur_ms, wrap: Option<(loop_start_ms, loop_end_ms)>) -> Vec<f64>   // sorted ms
  1. clicks = the beats of map.beats_in(0, dur_ms) whose section (segment_at, else 0) has meta.metronome
  2. for each section k >= 1 with meta.count_in, in ascending order:
       b  = 60000 / bpm_k
       F  = the first beat of section k (beats_in from start_k - 1e-6), = start_k for detected and placed changes
       if F >= dur_ms: skip
       if wrap = (ls, le) and F - 3.5*b < le <= F + 1e-6: skip      // a repeating loop ends before F (10.6)
       remove every click t with F - 3.5*b < t < F - 1e-6
       add F - 3b, F - 2b, F - b (those >= 0)
  3. sort
```

The manager turns the result into frames (rounded), removes duplicate frames and sends it with `Cmd::SetGrid`.
`SetMetronome(master on && tempo)` is unchanged.

**What "count-in for a section" means while playing through.** It is the same pattern as the Play count-in
(gui-play-backing-track req. 15: three clicks, the music starts on the silent fourth beat), placed just before the
section:
- three clicks one beat apart **at the next section's tempo**, the last one a beat before the section's first beat;
- the section's first beat is the fourth beat. It clicks only if that section's metronome is on, just as after a Play
  count-in.

**How the previous section is adjusted** (the new criterion). The previous section's own clicks in that span are
removed, so its last bar becomes the count-in. The removal starts half a beat before the first count-in click, so no
two clicks ever sound closer than half a beat.
- Example (TS11), sections 120 BPM from 500 and 93 BPM from 8500 with count-in:
  - the 120-BPM clicks at 6500, 7000, 7500 and 8000 are replaced by clicks at 6564.52, 7209.68 and 7854.84;
  - then 8500 and every 645.16 ms.

**Edge cases** (recommended answers Q2, Q3):
- **The previous section's metronome is off:** the three count-in clicks still play over the silent section. This
  follows the existing rule that a count-in plays even with the metronome off.
- **The previous section is shorter than the count-in:** the count-in still gives its three clicks. It reaches back
  over the earlier section(s), replacing their clicks too, and never goes before 0 ms.
- **Two count-ins overlap** (a short section with count-in followed by another with count-in): the later one wins.
  It is processed last and removes the earlier clicks inside its span.
- **The first section** has no section before it: its count-in is refused and its button disabled. Use Count In
  (Play) for the start of the song.
- Count-in clicks are part of the click list, so they follow the same speed rule as all clicks (speed is 1.0 until
  speed change exists).

### 10.5 The master switch and the Play count-in (recommended answers Q4, Q5)

| Control | Where | Effect |
|---|---|---|
| Metronome On/Off (master, top-level `metronome.on`) | Sections group, "Metronome" sub-group | Off: no clicks at all while the song plays, including section count-ins. On: the click list of 10.4 sounds |
| Count In (top-level `metronome.count_in`) | same sub-group | unchanged: the pre-roll before Play and before loop Play |
| Click (this section's `meta.metronome`) | Sections group, "This section" sub-group | whether this section's beats click |
| Count In (this section's `meta.count_in`) | same sub-group | the count-in of 10.4 before this section; disabled on section 1 |

- A section's count-in only affects playing **through** its start. Starting Play inside or at that section, or a loop
  starting there, keeps today's rule: the pre-roll comes from Count In or the loop's Count-In.
- **Why keep the master:** one press silences everything while playing. Without it, the user would have to turn every
  section off, and the existing "Metronome On/Off" tests, keys and `player.json` meaning stay valid.
- **Why compute clicks in the manager:** the engine and its real-time rules are untouched, and the click list is a pure
  function that is easy to test exactly. Each edit or toggle is one `SetGrid`. The engine's `beat_dirty` re-finds the
  next click at the current position, so changes are heard from the next beat, as for phase-1 edits. The cost is a
  `beats_in` over the whole track (3 000 beats for 10 min at 300 BPM) per change: negligible.

### 10.6 Loops spanning sections (recommended answer Q6)

The click list is per source frame, so a loop that crosses a section start plays that start's count-in on every pass.

The exception is a **repeating loop** (the current, complete loop with Loop On) that **ends at or just before** a
section start with count-in, with its end inside the count-in span. There the count-in would lead nowhere: playback
jumps back to the loop start, at the old tempo. A common case is a loop over exactly one section.
- That count-in is skipped (the `wrap` argument), and the section's own clicks stay.
- The manager keeps the `wrap` it last used (`Session::grid_wrap`). `sync_loop` re-sends the grid when the current
  repeating loop or its ends change, e.g. Loop On toggled, loop selected, marks moved.
- A loop played once with loop Play (no Loop On) keeps the count-in: playback stops at its end anyway.

### 10.7 Start / End (spec req. 10)

- **Target:** the section of 2.6 (the selection, else the section at the playhead).
- **Start** jumps to the section's start: its `start_ms`, or 0 ms for section 1 (the waveform draws section 1 from the
  left edge).
- **End** jumps to the **start of the section's last bar** (recommended answer Q7): the 4th beat (`beats_per_bar`) before
  the next change, or before the track end for the last section, but never before the section start.
  - The exact end is the next section's first beat, so jumping there would make the playhead (and the target) the
    next section.
  - From the last bar you hear the transition, and the count-in if there is one.
  - Example: 120 BPM from 500 to 8500 → End = 6500.
- **While playing:** an instant jump, as loop Prev/Next (`Cmd::Jump`: 3 ms fade-in, no scrub silence, playback goes
  on, a pre-roll count-in in progress is cancelled). **While paused:** the playhead moves, and the next Play applies the
  normal count-in rule.
- The selection is not changed. Pressing Start again restarts the same section, which is handy for practice.
- **Rust:** `PlayerManager::jump_section(id, index, at, edge, now)`:
  - the stale guard;
  - `TempoMap::section_jump_ms(index, edge, end_ms)`;
  - `set_pos` + `Cmd::Jump` + `mark_dirty` + `publish`;
  - log `calliope: player section jump id=… index=… edge=start|end position_ms=…`.
- **Keys:** Home = Start and End = End, as Player keys. They work wherever the Player keys work today (not while a
  field, button or other control that keeps its keys has the focus), and also on the focused waveform (`PlayerView`
  lets them through from `[data-player-keys]`, like Space).
  - `preventDefault`, so the pane does not scroll.
  - Auto-repeat is ignored.

### 10.8 Names (spec req. 11)

- **The name box** is a text input right after the status line ("Section 6 of 8 · 93 BPM").
  - Placeholder "Name this section", accessible name "Section name", `maxlength` 40.
  - It shows the target section's name.
- **The section is captured on focus:** `pl.nameEdit = {index, at, draft}`. While the box has the focus, the box, the
  status line and the section toggles show the captured section, so a playhead crossing into the next section can't
  move the edit.
- **Commit:** Enter or blur sends `section-name` with the captured `index`/`at` and `cleanSectionName(draft)`. Nothing
  is sent when the name is unchanged. An empty name clears it.
- **Escape** restores the old name and blurs.
- **A refusal** (stale, or invalid in Rust) shows in the Sections message line.
- **Keys:** typed keys never reach the Player keys (inputs keep their keys, `OWN_KEYS`). So Space, b, e and Home type
  as usual.
- **Rust:** `set_section_name(index, name)`:
  - trims the name;
  - refuses control characters ("Section names can't contain control characters");
  - refuses more than 40 characters ("Section names can have at most 40 characters").

  The frontend cleans first, so users never see these messages.
- **The waveform:** labels stay "93 BPM" (no layout risk on the canvas). The waveform's `aria-valuetext` gains the name:
  "Section 2 of 3, Verse, from 0:08.500, 93 BPM, …".

### 10.9 IPC (phase 2)

```ts
interface TempoSegment { start_ms: number; bpm: number; detected_ms?: number | null; on_beat?: boolean;
                         name?: string; metronome?: boolean; count_in?: boolean }   // missing = '', true, false
type TempoAction = /* phase 1 kinds */
  | { kind: 'section-flags'; index: number; at: number; metronome: boolean; count_in: boolean }
  | { kind: 'section-name'; index: number; at: number; name: string }
  | { kind: 'jump-section'; index: number; at: number; edge: 'start' | 'end' };
```

**Rust:**
- `player_ipc::TempoAction` gains `SectionFlags`, `SectionName { name: String }` and `JumpSection { edge: Edge }`.
  `Edge` is the loops' existing kebab-case enum. `TempoAction` and `SectionEdit` lose `Copy`.
- `section-flags` and `section-name` become `SectionEdit::Flags` and `SectionEdit::Name` in `edit_sections` (the same
  path: guard, new map, `sync_grid`, `sync_loop`, `mark_dirty`). `jump-section` calls `jump_section`.

**Log lines:**
- **Rust:** `calliope: player section <flags|name> id=… index=… segments=<k> bpm=<..> metronome=<1|0,..>
  count_in=<1|0,..>`. The two new fields are appended to every `player section` line.
- **Frontend:**
  - `calliope-ui: player sections …` gains ` metronome=<1|0,..> count_in=<1|0,..> named=<n>`;
  - `player section flags index=<i> metronome=<b> count_in=<b>`;
  - `player section name index=<i> length=<n>` (the name itself is not logged);
  - `player section jump index=<i> edge=<start|end> position=<ms>`.

### 10.10 The Sections group and the control bar layout

The Metronome group disappears. Its two buttons move into the Sections group as a nested `role="group"
aria-label="Metronome"`, so the existing tests that find "On/Off" and "Count In" in the Metronome group keep working.

```text
┌ Sections ─ Section 2 of 3 · 93 BPM  [Verse_____________] ───────────────────────────┐
│ [Add] [Remove] [×2] [÷2]   [|◀ Start] [End ▶|]                                       │
│ Metronome [On/Off] [Count In]     This section [Click] [Count In]                    │
│ <message line, reserved>                                                             │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

- **Buttons:** 3.25 rem high, toggles filled amber when on (as today).
  - The section toggles are "Click" (accessible name "Metronome in this section") and "Count In" ("Count-in into this
    section").
  - Start is "Jump to the start of the section", End is "Jump to the last bar of the section".
  - Disabled ones have a `title` saying why: BPM 0 → "Type a BPM first"; section 1 Count In → "The first section has
    no section before it".
- **Space:** at 1280x800 with the tree folded, phase 1 left BPM, Sections and Metronome in the first row and the pane
  scrolling by about 14 px. The title and the status now share one line, and the Metronome group's border, padding and
  title are gone; that pays for most of the new button row.
- **The target:** the pane scrolls no more than in phase 1 at 1280x800 with the tree folded (the visual review compares
  it with the phase-1 `player-sections` screenshot). With the tree shown or at 1024x640 the groups wrap and the pane
  scrolls, as before.
  - If the target can't be met, the visual review may put the Start / End buttons on the status line (the name box
    narrows to 10 rem). It reports which layout it chose.
- The GUI tests' fixed click positions are re-measured from screenshots (Task 22).

### 10.11 The Editor

No Editor view change: it shows sections and their BPMs as today, without names.
- It keeps names and flags when it moves a detected change: `set_change_offset` keeps the segment, and
  `TempoStore::replace` writes the segments with their meta.
- Autodetect replaces the map (Q8).
- The `TempoSegment` type is shared, and the Editor ignores the new keys.

## 11. Tasks (phase 2)

Same rules as section 3: Rust tasks end with `cargo clippy --workspace --all-targets -- -D warnings` clean, frontend
tasks with `npm run check` clean (both in `src/calliope-gui/`).

### Task 12: Tempo map: `SectionMeta`, carry rules, flags, names, jump positions
- **files:** `src/calliope-gui/src/tempo_map.rs`. Compiler-driven `meta: SectionMeta::default()` additions to the
  `Segment { .. }` literals in:
  - `src/editor_tests.rs`, `src/player_ipc_tests.rs`, `src/player_state.rs`, `src/player_tests.rs`;
  - `src/tempo_detect.rs`, `src/tempo_detect_tests.rs`, `src/tempo_store.rs`;
  - `tests/acceptance_beat_autodetect.rs`, `tests/acceptance_player_backend.rs`.
- **does:**
  - `SectionMeta` and the `meta` field (10.2), with `fn yes()` and `fn is_true()` next to `is_false`.
  - The carry rules (10.3) in `add_change`, `remove_change` (nothing to do but a test) and `set_bpm_at` (insert: copy the
    split section's `metronome`; first segment removed: clear the new first's `count_in`).
  - `set_section_flags(index, metronome, count_in) -> Result<TempoMap, String>`: index out of range → "There is no such
    section"; `count_in` on index 0 → "The first section has no section before it".
  - `set_section_name(index, name) -> Result<TempoMap, String>` (10.8).
  - `section_jump_ms(index, edge: Edge, end_ms) -> Result<f64, String>` (10.7). Define `pub enum Edge { Start, End }` in
    `tempo_map.rs`; `player_ipc` maps its own `Edge` to it.
- **tests** (in `tempo_map.rs`):
  - Serde: a default meta adds no keys, so a schema-3 map serialises byte-identically; `name`, `metronome: false` and
    `count_in: true` round-trip.
  - Add copies the split section's `metronome` (on and off) and gives no name or count-in. Remove keeps the left
    section's meta. Move, section BPM and scale keep the meta. `set_bpm_at` (insert, merge, BPM 0 on the first) follows
    10.3.
  - Flags and name refusals, with the exact messages. A 40-character name is accepted, 41 refused; "  Verse " → "Verse".
  - `section_jump_ms` on `[(500,120),(8500,93)]` with end 20000:
    - Start 0 → 0, Start 1 → 8500;
    - End 0 → 6500;
    - End 1 → the 4th beat before 20000 on the 93-BPM grid from 8500;
    - a 2-beat section → End = its start.
- **done when:** `cargo test -p calliope-gui --lib tempo_map` and `cargo test --workspace --no-run` pass; clippy clean.
- **routine:** no

### Task 13: `player.json` schema 4 (code and its own tests)
- **files:** `src/calliope-gui/src/player_state.rs`, `src/calliope-gui/src/tempo_store.rs`
- **does:**
  - `SCHEMA = 4`.
  - `parse_tempo` reads `name`, `metronome` and `count_in` as in 10.2 and clears `count_in` on the first segment.
  - Reads 1-4; 5+ read-only.
  - In these two files only:
    - "written as 3" asserts become 4 (`player_state.rs`: `back["schema_version"], 3`, the `v["schema_version"] =
      json!(3)` that follows it, and `v["schema_version"], 3` near line 503);
    - "newer schema" fixtures `4` become `5` (`player_state.rs` near line 563, `tempo_store.rs:100`).

    Fixtures that read schema 3 stay 3.
- **new tests** (`player_state.rs`):
  - A v3 file with `on_beat` and `detected_ms` loads with every section metronome on, no count-in and no name. It is
    written as 4 with the same segments.
  - A v4 segment with all three keys round-trips.
  - `count_in` on the first segment is dropped.
  - Bad values are ignored and the segment is kept: `"name": 5`, `"metronome": "no"`, `"count_in": 1`.
  - A name with a tab and 50 characters is read cleaned and cut to 40.
- **done when:** `cargo test -p calliope-gui --lib player_state` and `cargo test -p calliope-gui --lib tempo_store`
  pass; clippy clean.
- **routine:** no

### Task 14: Schema-number literals in the other tests
- **files:** `src/calliope-gui/src/beat_detect_tests.rs`, `src/calliope-gui/tests/acceptance_beat_autodetect.rs`,
  `src/calliope-gui/tests/acceptance_player_backend.rs`, `src/calliope-gui/tests/gui_editor_e2e.rs`,
  `src/calliope-gui/tests/gui_player_e2e.rs`
- **does:** exact edits. Change only the lines listed; line numbers are from base `22bc967` and may be a few lines off,
  so find each by its text:
  1. "written as 3" becomes "written as 4": replace `3` with `4` in each of these lines:
     - `beat_detect_tests.rs:146`: `assert_eq!(r.player_json().unwrap()["schema_version"], 3);`
     - `acceptance_beat_autodetect.rs:460` and `:713`: `assert_eq!(v["schema_version"], 3);`
     - `acceptance_player_backend.rs:1196`: `assert_eq!(v["schema_version"], 3);`
     - `acceptance_player_backend.rs:1770` and `:2044`: `assert_eq!(r.disk().unwrap()["schema_version"], 3);`
     - `acceptance_player_backend.rs:1984` and `:2014`: `assert_eq!(d["schema_version"], 3);`
     - `gui_editor_e2e.rs:361`, `gui_player_e2e.rs:356` and `:425`: `assert_eq!(v["schema_version"], 3, "{v}");`
  2. "newer schema" fixtures: replace `"schema_version":4` with `"schema_version":5` in:
     - `acceptance_beat_autodetect.rs:611` (`let newer = r#"{"schema_version":4,…`);
     - `acceptance_player_backend.rs:1259` (`let text = r#"{"schema_version":4,"source":…`);
     - `acceptance_player_backend.rs:2049` (`let text = r#"{"schema_version":4,"tempo":…`).
  3. Leave every other `schema_version` alone (fixtures that read schema 1, 2 or 3; `track.json` literals;
     `gui_player_acceptance_e2e.rs`' schema 7).
- **done when:** `cd src/calliope-gui && cargo test -p calliope-gui --lib beat_detect && cargo test -p calliope-gui
  --test acceptance_player_backend --test acceptance_beat_autodetect && cargo test -p calliope-gui --features e2e-hooks
  --test gui_editor_e2e --test gui_player_e2e --no-run` passes. Run it once, then stop.
- **routine:** yes

### Task 15: The click list
- **files:** `src/calliope-gui/src/metronome.rs`
- **does:** `pub(crate) fn section_clicks(map: &TempoMap, dur_ms: f64, wrap: Option<(f64, f64)>) -> Vec<f64>` exactly
  as 10.4. Empty map → empty.
- **tests** (in `metronome.rs`), within 1e-6 ms:
  - Default meta: equal to `beats_in(0, dur)`'s times, so phase 1 is unchanged.
  - Section metronome off: no clicks in that section, the others unchanged; the beats before the first start belong to
    section 0.
  - TS11 numbers: `[(500,120),(8500,93 detected, count_in)]` → …6000, 6564.516, 7209.677, 7854.839, 8500, 9145.161…;
    no click in (6241.94, 8500) other than those three.
  - Previous section off: the same three clicks, nothing else before 8500 after 0.
  - Typed (tick-rule) change with count-in: anchored on its first beat, not on `start_ms`.
  - Short previous section: `[(0,120),(4000,120 on_beat),(5000,60 on_beat, count_in)]` → clicks 2000, 3000, 4000, then
    5000; 1500 is kept and 2500, 3500 and 4500 are gone.
  - Two overlapping count-ins: the later one wins.
  - `wrap = Some((500, 8500))` → no count-in for 8500, and the 120-BPM clicks stay. `wrap = Some((500, 6000))` (ends
    before the span) → the count-in is kept.
  - Count-in clicks before 0 ms are dropped. A section starting at or after `dur_ms` is ignored.
  - Always sorted and strictly increasing.
- **done when:** `cargo test -p calliope-gui --lib metronome` passes; clippy clean.
- **routine:** no

### Task 16: Player session: click grid, flags, names, jumps, IPC
- **files:** `src/calliope-gui/src/player.rs`, `src/calliope-gui/src/player_tests.rs`,
  `src/calliope-gui/src/player_ipc.rs`, `src/calliope-gui/src/player_ipc_tests.rs`
- **does:**
  - **The grid:**
    - `Session::grid()` uses `section_clicks(&tempo, dur_ms, self.wrap_ms())`, rounds to frames and drops duplicate
      frames (accent `false`);
    - `wrap_ms()` = the current complete loop's (start, end) ms when its `repeat` is on, else `None`;
    - `Session::grid_wrap` holds the value last sent;
    - `sync_loop` becomes `&mut self` and calls `sync_grid` when `wrap_ms()` differs from `grid_wrap` (10.6).
  - **`SectionEdit`:** gains `Flags { index, at, metronome, count_in }` and `Name { index, at, name: String }` (drop
    `Copy`), handled in `edit_sections` with the guard. Log kinds `flags` and `name`. Every `player section` line gets
    ` metronome=… count_in=…` (10.9).
  - **`jump_section`** (10.7).
  - **`player_ipc`:** the three `TempoAction` variants (drop `Copy`), mapped in `do_player_tempo`.
- **tests:**
  - **`player_tests.rs`** (ManualBackend; reuse the phase-1 helpers):
    - toggling a section's metronome off while playing → no click from the next beat on in that section; on again →
      clicks from the next beat;
    - flags and name are on disk after `tick(t0+500ms)` and not at `t0+499`;
    - stale `at` → the exact message, nothing changes;
    - Loop On toggled on a loop ending at a count-in section start → the next pass has no count-in clicks; Loop On off
      → they are back;
    - master Off → no clicks, including count-ins; Play's pre-roll with Count In still plays (unchanged);
    - `jump_section` while playing → no pause or seek event, the position is at the target frame and the following
      audio equals the source from there (after the 3 ms fade); while paused → the position moves and nothing sounds;
    - jump during a pre-roll count-in → the count-in is cancelled, as for loop Next.
  - **`player_ipc_tests.rs`:** the JSON of each new action deserialises (`{"kind":"section-name","index":1,"at":8500.0,
    "name":"Verse"}`, `{"kind":"jump-section","index":0,"at":500.0,"edge":"end"}` …); an unknown edge is rejected.
- **done when:** `cargo test -p calliope-gui --lib player` passes; clippy clean.
- **routine:** no

### Task 17: Backend acceptance tests for phase 2
- **files:** `src/calliope-gui/tests/acceptance_player_backend.rs`
- **does:**
  - A new section "section metronome (gui-player-tempo-sections phase 2)". Reuse `rig_with`, `clicks`, `onsets` and the
    TS fixture, written as schema 4 with the needed keys.
  - **Tests:**
    - `ts11_count_in_replaces_the_previous_sections_last_clicks`: section 1 `count_in: true`, master on, play from
      5000 → onsets 5000, 5500, 6000, 6564.52, 7209.68, 7854.84, 8500, 9145.16 (±1 frame), nothing else in between.
    - `ts12_section_metronome_off_is_silent_and_follows_while_playing`: section 0 `metronome: false` → no click before
      8500, clicks at 93 after; `section-flags` on while playing → clicks from the next 120-BPM beat; master Off →
      silence.
    - `ts13_count_in_with_previous_off_short_previous_and_loops`:
      - previous section off → the three clicks only;
      - a short previous section (fixture as Task 15) → the count-in reaches back;
      - a Loop On loop 500..8500 → no count-in at its end, and the 120-BPM clicks play.
    - `ts14_jump_to_section_start_and_last_bar`: playing → `jump-section` start 1 → position 8500, playback continues
      and the audio equals the source; end 0 → 6500; paused → the position moves only.
    - `ts15_names_and_flags_survive_close_and_reopen`: name, flags and the phase-1 edits; `close`; reopen → the same
      segments with their meta; schema 4.
    - `ts16_older_schemas_upgrade`: v1, v2 and v3 files → all metronome on, no count-in, no name, the clicks as before;
      written as 4; a schema-5 file is never overwritten through flags, name or jump.
    - `ts17_edits_carry_the_section_settings`: Add copies the metronome flag; Remove drops the joined section's name;
      move, BPM and scale keep everything; the Editor's `set_change_offset` + `TempoStore::replace` path keeps names
      and flags in the file.
- **done when:** `cargo test -p calliope-gui --test acceptance_player_backend ts1` passes (it runs ts1 and ts11-ts17) in
  under 20 s.
- **routine:** no

### Task 18: TS IPC types for phase 2
- **files:** `src/calliope-gui/src/ui/lib/ipc.ts`, `src/calliope-gui/src/ui/lib/ipc-player.test.ts`
- **does:** exact edits:
  1. In `ipc.ts`, in `interface TempoSegment`, directly after the two lines
     ```ts
       /** set on changes added or moved in the Player: the section starts on a beat */
       on_beat?: boolean;
     ```
     add
     ```ts
       /** the section's name; missing = no name */
       name?: string;
       /** the section's metronome; missing = on */
       metronome?: boolean;
       /** a count-in before this section while playing through its start; missing = off */
       count_in?: boolean;
     ```
  2. In `ipc.ts` replace the line
     ```ts
       | { kind: 'move-change'; index: number; at: number; steps: number };
     ```
     with
     ```ts
       | { kind: 'move-change'; index: number; at: number; steps: number }
       | { kind: 'section-flags'; index: number; at: number; metronome: boolean; count_in: boolean }
       | { kind: 'section-name'; index: number; at: number; name: string }
       | { kind: 'jump-section'; index: number; at: number; edge: 'start' | 'end' };
     ```
  3. In `ipc-player.test.ts`, insert directly before the file's last line (the `});` closing `describe('player ipc
     wrappers', …)`):
     ```ts
       it('player_tempo passes the phase-2 section actions unchanged', async () => {
         const seen: unknown[] = [];
         mockIPC((cmd, args) => {
           seen.push([cmd, args]);
           return null;
         });
         await playerTempo('t1', { kind: 'section-flags', index: 1, at: 8500, metronome: false, count_in: true });
         await playerTempo('t1', { kind: 'section-name', index: 1, at: 8500, name: 'Verse' });
         await playerTempo('t1', { kind: 'jump-section', index: 0, at: 500, edge: 'end' });
         expect(seen).toEqual([
           ['player_tempo', { id: 't1', action: { kind: 'section-flags', index: 1, at: 8500, metronome: false, count_in: true } }],
           ['player_tempo', { id: 't1', action: { kind: 'section-name', index: 1, at: 8500, name: 'Verse' } }],
           ['player_tempo', { id: 't1', action: { kind: 'jump-section', index: 0, at: 500, edge: 'end' } }],
         ]);
       });
     ```
     Nothing else changes; the file must still end with a newline.
- **done when:** `cd src/calliope-gui && npx vitest run src/ui/lib/ipc-player.test.ts && npm run check` passes. Run it
  once, then stop.
- **routine:** yes

### Task 19: `section-meta.ts`
- **files:** `src/calliope-gui/src/ui/lib/section-meta.ts` (new), `src/calliope-gui/src/ui/lib/section-meta.test.ts` (new)
- **does:** create the two files with exactly this content.
  `section-meta.ts`:
  ```ts
  import type { TempoSegment } from './ipc';

  /** Mirrors the Rust limit in `set_section_name` (characters, not bytes). */
  export const MAX_SECTION_NAME = 40;

  /** The section's metronome; a missing key means on (files older than schema 4). */
  export function sectionClicks(seg: TempoSegment): boolean {
    return seg.metronome !== false;
  }

  /** The section's count-in; a missing key means off. */
  export function sectionCountIn(seg: TempoSegment): boolean {
    return seg.count_in === true;
  }

  /** The name as Rust accepts it: control characters removed, trimmed, at most MAX_SECTION_NAME characters. */
  export function cleanSectionName(raw: string): string {
    const kept = Array.from(raw).filter((c) => {
      const n = c.codePointAt(0) ?? 0;
      return n >= 0x20 && !(n >= 0x7f && n <= 0x9f);
    });
    return Array.from(kept.join('').trim()).slice(0, MAX_SECTION_NAME).join('').trim();
  }
  ```
  `section-meta.test.ts`:
  ```ts
  import { describe, expect, it } from 'vitest';
  import { cleanSectionName, MAX_SECTION_NAME, sectionClicks, sectionCountIn } from './section-meta';

  describe('section meta', () => {
    it('reads the flags with their defaults', () => {
      expect(sectionClicks({ start_ms: 0, bpm: 120 })).toBe(true);
      expect(sectionClicks({ start_ms: 0, bpm: 120, metronome: false })).toBe(false);
      expect(sectionCountIn({ start_ms: 0, bpm: 120 })).toBe(false);
      expect(sectionCountIn({ start_ms: 0, bpm: 120, count_in: true })).toBe(true);
    });
    it('cleans names like Rust does', () => {
      expect(cleanSectionName('  Verse ')).toBe('Verse');
      expect(cleanSectionName('Ch\torus\n')).toBe('Chorus');
      expect(cleanSectionName('')).toBe('');
      expect(cleanSectionName('x'.repeat(50))).toHaveLength(MAX_SECTION_NAME);
      expect(cleanSectionName('é'.repeat(41))).toBe('é'.repeat(40));
    });
  });
  ```
- **done when:** `cd src/calliope-gui && npx vitest run src/ui/lib/section-meta.test.ts && npm run check` passes. Run
  it once, then stop.
- **routine:** yes

### Task 20: Player frontend state for flags, names and jumps
- **files:** `src/calliope-gui/src/ui/lib/player-state.svelte.ts`, `src/calliope-gui/src/ui/lib/player-state.test.ts`
- **does:**
  - **`setSectionFlags(metronome, countIn)`** on the target (or the captured name-edit section), sending `at` as in
    phase 1.
  - **The name edit:** `pl.nameEdit: {index, at, draft} | null`; `beginNameEdit()` (on focus), `commitSectionName()`,
    `cancelSectionName()` (10.8). Cleared on track change, like the selection.
  - **`jumpSection(edge)`** on the target.
  - **Errors** go to `pl.sectionError`, and success clears it.
  - **Log lines** (10.9).
- **tests** (mockIPC):
  - each action's exact `player_tempo` arguments;
  - the name edit keeps its section when `transport.tempo_index` changes during the edit;
  - unchanged → no call; Escape → no call and the old name; a stale refusal → `sectionError`;
  - `jumpSection` uses the selection, else `tempo_index`;
  - the `player sections` log line has the new fields.
- **done when:** `npx vitest run src/ui/lib/player-state.test.ts` passes; `npm run check` is clean.
- **routine:** no

### Task 21: Sections group with the Metronome controls, name box, toggles, Start / End, keys
- **files:**
  - `src/calliope-gui/src/ui/components/player/SectionsSection.svelte`;
  - `src/calliope-gui/src/ui/components/player/MetronomeSection.svelte` (delete);
  - `src/calliope-gui/src/ui/components/player/PlayerPane.svelte`, `src/calliope-gui/src/ui/views/PlayerView.svelte`;
  - `src/calliope-gui/src/ui/components/player/WaveformView.svelte` (only the `aria-valuetext` name);
  - `src/calliope-gui/src/ui/components/player/ControlBar.test.ts`, `src/calliope-gui/src/ui/acceptance_player.test.ts`.
- **does:**
  - The group of 10.10, wired to player-state. The master buttons keep their names inside the nested "Metronome" group.
  - `PlayerPane` drops `MetronomeSection`.
  - `PlayerView`: Home / End (10.7), let through from `[data-player-keys]`.
  - The waveform's `aria-valuetext` includes the name.
- **tests:**
  - **`ControlBar.test.ts`:**
    - the toggles' pressed state comes from the target's meta;
    - section 1 → Count In disabled with its title;
    - clicks → the right `section-flags`;
    - the name box shows the target's name, and Enter / blur / Escape behave as in 10.8;
    - Start / End → `jump-section`;
    - the master buttons work as before;
    - the message line keeps its height.
  - **`acceptance_player.test.ts`:**
    - the existing Metronome tests pass unchanged (the nested group);
    - Home / End jump with the focus on the page and on the waveform, and type normally in the name box;
    - Space in the name box types a space and does not play.
- **done when:** `npx vitest run src/ui/components/player/ src/ui/acceptance_player.test.ts src/ui/views/` passes;
  `npm run check` is clean.
- **routine:** no

### Task 22: GUI end-to-end, layout review, docs
- **files:** `src/calliope-gui/tests/gui_player_e2e.rs` (and any other Player GUI test with fixed click positions, e.g.
  `tests/gui_player_acceptance_e2e.rs`), `docs/ui.md`, `src/calliope-gui/README.md`
- **does:**
  - **Positions:** re-measure the fixed click positions (`METRO`, `METRO_COUNT_IN`, `SEC_*`, `PLAY`, `LOOP_*` …) at
    1280x800 with the tree folded, from screenshots, and add `SEC_NAME`, `SEC_CLICK`, `SEC_COUNT_IN`, `SEC_START` and
    `SEC_END`.
  - **New test `player_section_metronome`.** Capture as in `tempo_metronome_count_in`. Before start, write the TS
    fixture `player.json` (schema 2). Steps:
    1. pick the track → `player sections segments=2 … metronome=1,1 count_in=0,0`; screenshot `player-sections-p2`;
    2. click section 1 on the waveform, then Count In (section) → `player section flags index=1 metronome=true
       count_in=true`;
    3. Metronome On, seek 5.0 s, Play, about 4.5 s, Pause → capture onsets 6000, 6564.5, 7209.7, 7854.8, 8500 (±5 ms)
       and none in between;
    4. click section 0 on the waveform, then Click → `metronome=false`; seek 5.0 s, Play, Pause → no onset before
       6564.5, the three count-in onsets, then 8500;
    5. select section 1, type "Verse" in the name box, Enter → `player section name index=1 length=5`; screenshot
       `player-section-name`;
    6. End → `player section jump index=1 edge=end position=<the last-bar start>`; click `BLANK` (so no button has
       the focus), then the Home key → `edge=start position=8500` (section 1 is still selected);
    7. after 0.6 s, `player.json` is schema 4, with `segments[1]` `name` "Verse" and `count_in` true, and
       `segments[0].metronome` false;
    8. restart the app on the same dirs and pick the track → `… metronome=0,1 count_in=0,1 named=1`, and the name box
       shows "Verse" with section 1 selected.
  - **`visual_review`** gains "section toggles on + name typed" (both themes; 1280x800 and 1024x640; tree shown and
    folded). Review:
    - the scroll target of 10.10 against the phase-1 `player-sections` shot;
    - legibility of the name box and the two "Count In" buttons, which must be clearly told apart by their sub-group
      captions;
    - nothing moves when the message line fills.
  - **Docs:**
    - `docs/ui.md` "Decided by the team": the merged group, the toggles, the name box rules, Start / End and Home /
      End; update the layout paragraphs that list a Metronome group;
    - README usage.
- **done when:**
  - `cd src/calliope-gui && npm run test:gui -- player_section_metronome` passes (private display);
  - the screenshots in `target/gui-shots/` were reviewed and the layout choice is reported;
  - `npm test` and `npm run test:gui` pass.
- **routine:** no

**Order (phase 2):**
- Rust: 12 → 13 → 14; 12 → 15 → 16 → 17 (14 before 17).
- Frontend: 18 and 19 any time; 18 → 20 → 21.
- Last: 22.

## 12. Test strategy (phase 2 additions)

- **Pure Rust:**
  - `SectionMeta` serde and the carry rules, flags, names, `section_jump_ms` (Task 12);
  - schema 4 read/write and upgrade (Task 13);
  - `section_clicks` with every edge case of 10.4 and 10.6 (Task 15). This is where the count-in rule is pinned
    exactly.
- **Player session** (`player_tests`, ManualBackend, injected `Instant`s): grid re-upload on flags and on loop changes,
  next-beat behaviour, master switch, jumps, stale guard, debounced writes (Task 16).
- **Audio behaviour** (`acceptance_player_backend`, ManualBackend): click onsets at exact frames for TS11-TS13, jump
  continuity and source equality for TS14, reopen from disk for TS15-TS17 (Task 17). No real device: `CpalBackend` is
  never constructed in tests.
- **Frontend:** vitest + mockIPC for the helpers, the state, the group and the keys (Tasks 18-21); `svelte-check`.
- **GUI:** `player_section_metronome` on a private display with `CALLIOPE_E2E_AUDIO=capture:` onset analysis, plus the
  screenshots and the layout review (Task 22).
- **Not involved:** calliope-stems, the network, archserver, other hosts and any real gear.
- **Whole suite (unchanged):** `cd src/calliope-gui && npm test`, then `npm run test:gui`.

## 13. Deployment (phase 2)

Laptop only, as in section 5: `cd src/calliope-gui && npm run build:app` and install.

**Note:** this build writes `player.json` as schema 4. The phase-1 build (schema 3) opens such files read-only. It plays
with defaults and never overwrites them, so don't mix builds on the same repository.

## 14. Manual checks (phase 2, owner: laptop with real audio)

- **M8: Count-in into a section.**
  1. On Deja-Vu, select the section after the intro and turn its Count In on. Metronome On, Play from a few bars
     before it.
  2. The last bar before the change counts "1, 2, 3" at the new tempo, and the new section starts on the next beat.
  3. It sounds right musically. If it doesn't, answer Q1 again.
- **M9: Sections without clicks.**
  1. Turn Click off for the intro: the intro is silent, and the clicks start with the next section.
  2. With the intro's Click off and the next section's Count In on, the count-in still plays over the intro.
  3. Metronome On/Off silences everything.
- **M10: Practising one section.**
  1. Make a loop over exactly one section whose next section has Count In, and turn Loop On.
  2. No count-in plays at the loop end. Played through without the loop, it does.
- **M11: Start / End.** While playing, press End (or the End key): playback jumps to the section's last bar without a
  gap. Start (Home) jumps back to its first beat. Paused, they only move the playhead.
- **M12: Names.**
  1. Name three sections ("Intro", "Verse", "Chorus"). While playing, the name box shows the section under the
     playhead, and it doesn't jump while you type.
  2. Quit and restart: names and toggles are back.
  3. The Editor shows the sections as before, with no names.
- **M13: Layout.** At your usual window size the Sections group is readable at 1-2 m, the two Count In buttons are easy
  to tell apart, and Play is still where your hand expects it.

## 15. Acceptance mapping (phase 2)

| # | Criterion | Spec | Tasks | Tests |
|---|---|---|---|---|
| TS11 | Given a section's count-in is enabled, when a previous section is present and has its metronome active, then that previous section's last clicks are replaced by the count-in: three clicks at the next section's tempo, the last one a beat before its first beat | new criterion, req 9 | 12, 15, 16, 17, 22 | metronome (`section_clicks`), `ts11_*`, gui `player_section_metronome` step 3 |
| TS12 (added) | Given a section with its metronome off, then no click sounds in it and the other sections are unchanged; a toggle while playing takes effect from the next beat; Metronome On/Off silences all clicks | req 8 | 12, 15, 16, 17, 21, 22 | metronome, player_tests, `ts12_*`, ControlBar.test, gui step 4 |
| TS13 (added) | Given a section with count-in, then the count-in also plays when the previous section's metronome is off, reaches back when the previous section is shorter, is skipped at the end of a repeating loop that ends at that section, and the Metronome controls are in the Sections group | req 9 | 15, 16, 17, 21 | metronome, player_tests, `ts13_*`, acceptance_player.test (Metronome group nested) |
| TS14 (added) | Given a target section, when the user presses Start or End (or Home / End), then the playhead jumps to its start or its last bar; while playing, playback continues without a pause | req 10 | 12, 16, 17, 20, 21, 22 | tempo_map (`section_jump_ms`), player_tests, `ts14_*`, ControlBar.test, acceptance_player.test, gui step 6 |
| TS15 (added) | Given a section, when the user types a name in the box next to "Section n of m · BPM", then it is saved, and names and metronome settings are restored from player.json after close and reopen | req 11 | 12, 13, 16, 17, 20, 21, 22 | player_state, `ts15_*`, player-state.test, ControlBar.test, gui steps 5, 7, 8 |
| TS16 (added) | Given a `player.json` of schema 1-3, then it loads with every section clicking, no count-in and no names, and is written as 4; a schema-5 file is never overwritten | 10.2 | 13, 14 | player_state, `ts16_*`, existing schema tests |
| TS17 (added) | Given Add, Remove, move, BPM or ×2/÷2, then names and flags follow 10.3; the Editor's view is unchanged and its move keeps them | 10.3, 10.11 | 12, 17 | tempo_map, `ts17_*`, editor suites kept green |

## 16. Owner decisions (phase 2; 2026-10-17: all recommended answers, Q1-Q10)

The owner kept every recommended answer below; the design above assumes them.

1. **What is a section's count-in while playing through its start?**
   - (a) The Play count-in pattern: three clicks one beat apart at the new section's tempo, the last one a beat before
     its first beat, which is the "silent 4th" and clicks only if that section's metronome is on. The previous
     section's clicks in that span are replaced.
   - (b) A full bar: four clicks at the new tempo before the change.
   - (c) No tempo preview: keep the old clicks and just accent the bar before.

   **Recommended: (a)**, the same feel as the existing count-in. (b) changes only Task 15's constants and tests.
2. **The previous section's metronome is off.** (a) The count-in still plays (a count-in already plays with the
   metronome off). (b) It plays only if the previous section clicks (the literal reading of the criterion).
   **Recommended: (a).**
3. **The previous section is shorter than the count-in.** (a) The count-in still gives three clicks, reaching back over
   earlier sections. (b) Only the clicks that fit inside the previous section. **Recommended: (a).**
4. **Does the global Metronome On/Off stay?** (a) Yes, as a master switch: Off silences everything that clicks during
   the song, section count-ins included. Count In stays the Play / loop pre-roll. (b) Remove it: only per-section
   toggles. (c) Keep it, but let section count-ins play even when it is Off. **Recommended: (a).** (c) needs an engine
   change.
5. **Does a section's count-in also apply when Play starts inside or at that section, or when a loop starts there?**
   (a) No: it is only for playing through the section start; pre-rolls stay with Count In and the loop's Count-In.
   (b) Yes: Play at a section with count-in plays the pre-roll too. **Recommended: (a)**: two clear controls instead of
   one with two meanings. The first section's count-in is disabled (it has no section before it).
6. **A repeating loop that ends at a section with count-in.** (a) No count-in at the loop end (it would lead back to
   the old tempo). (b) Play it anyway (simpler: no loop-dependent grid). **Recommended: (a).** (b) removes the `wrap`
   argument and the `grid_wrap` re-send from Tasks 15-16.
7. **Where does End jump?** (a) The start of the section's last bar. (b) The exact end, which is the next section's
   first beat, so the target becomes the next section. (c) Two beats before the end. **Recommended: (a).**
8. **The Editor** ("Changes to the Editor view need confirmation first"). Nothing in the Editor view changes, and names
   are not shown there. One data effect: Autodetect in the Editor replaces the whole map, so names and section toggles
   are lost, as the Player's BPM edits already are. (a) Accept. (b) Carry each old section's settings to the new section
   whose start is nearest, within 4 beats (Rust only, no view change). **Recommended: (a)** for now. Also confirm that
   showing names on the Editor's waveform is not wanted yet.
9. **A section created by Add (or by typing a BPM).** (a) It copies the split section's metronome on/off, with no name
   and no count-in. (b) It always starts with the metronome on. **Recommended: (a)**: nothing you hear changes until you
   change it.
10. **Smaller rules applied unless you say otherwise:**
    - names are at most 40 characters, trimmed, without control characters, and an empty box clears the name;
    - names are not drawn on the waveform labels (the box shows them; the labels stay "93 BPM");
    - Start on section 1 goes to 0:00;
    - the control bar's Metronome group merges into the Sections group (nested "Metronome" and "This section"
      sub-groups);
    - `player.json` becomes schema 4, which the phase-1 build opens read-only.

# Phase 3: Player layout, no master metronome, Play count-in per section, default loop end

Spec requirements 12-16 and four new criteria (commit `ce958d3`). Base: the finished phase 2 (tasks 12-22, `ce958d3`).
Owner clarifications (conversation, 2026-10-18): loops are **not** merged with tempo sections; the Play pre-roll is per
section (the section under the playhead has Count In → count-in at that section's tempo); the loop's Count-In toggle
stays. The owner's answers are recorded in section 24 (2026-10-18): Q1-Q7 as recommended, and a changed layout (Q8:
Sections and Loops side by side, Playback below).

## 17. Summary (phase 3)

- **Layout.** Under the waveform come two full-width rows:
  - row 1 holds **Sections** (the BPM group on the left, then the section controls in 3 rows) and **Loops** (unchanged
    controls, now with a visible "Loops" title) side by side;
  - row 2, at the very bottom, is **Playback** (Rew, Play, FF, time and the position slider in one row).
- **No master metronome.** Metronome On/Off and its Count In disappear, with the `player_metronome` command and the
  top-level `metronome` object of `player.json`. Only the per-section Click and Count In remain.
- **Play count-in per section.** Play counts in (three clicks at that section's tempo) when the section under the
  playhead has Count In on. Section 1 may now have Count In (it means only the Play count-in there). Loop Play counts in
  only with the loop's own Count-In.
- **Default loop end.** "+" (Add loop) sets the new loop's end to the start of the next tempo section after the start,
  or to the track end.
- `player.json` becomes **schema 5**. Older files are migrated so that what you hear stays the same (24, Q1).

## 18. Design (phase 3)

### 18.1 What changes

| Where | What | Pure? |
|---|---|---|
| `src/calliope-gui/src/tempo_map.rs` | count-in allowed on section 1; `count_in_at(ms)`; `next_section_tick(tick)`; `with_play_settings_of(old)` (18.6) | yes |
| `src/calliope-gui/src/player_state.rs` | schema 5; `Metronome` and `PlayerState.metronome` removed; migration of schema 1-4 (18.3); `add(position_ms, track_end_ms)` with the default end (18.5) | yes |
| `src/calliope-gui/src/player.rs` | no `set_metronome`; snapshot without `metronome`; engine metronome flag = "has a tempo"; Play pre-roll from `count_in_at`; loop Play pre-roll from the loop only; Add passes the duration | Player internals |
| `src/calliope-gui/src/player_ipc.rs`, `gui.rs`, `build.rs`, `capabilities/main.json` | the `player_metronome` command and its permission removed | wiring |
| `src/calliope-gui/src/beat_detect.rs` | Autodetect keeps the play settings (18.6) | - |
| `src/ui/lib/ipc.ts`, `player-state.svelte.ts` | `PlayerSnapshot.metronome`, `playerMetronome`, `setMetronome` removed; the loop log line gains ` end=` | types / frontend |
| `src/ui/components/player/SectionsSection.svelte` | holds `BpmSection` on the left; no Metronome sub-group; Count In enabled on section 1; message line in row 3 | component |
| `src/ui/components/player/BpmSection.svelte` | `nested` look (no card border of its own) | component |
| `src/ui/components/player/LoopsSection.svelte` | visible "Loops" title at the start of its first line; `flex-1` with a minimum width so it sits beside Sections; spacing only | component |
| `src/ui/components/player/PlaybackSection.svelte` | full width, one row (slider grows on the same row, wraps when narrow) | component |
| `src/ui/components/player/PlayerPane.svelte` | row 1 = Sections + Loops (wrapping), row 2 = Playback; the `player layout` log line (18.2) | component |

Unchanged: the engine (`Engine`, `Cmd`, `render`), `section_clicks` (count-ins while playing through, the loop-wrap skip),
the section edits and their IPC, the waveform, the Player keys, `LoopCmd`/`LoopAction` (no new action), the Editor view.

### 18.2 Layout (requirements 12, 13, 15; owner Q8, 2026-10-18)

```text
┌ Sections ──────────────────────────────────────────────────────┐ ┌ Loops [Prev] [+] [1/2] [-] [Next] ─┐
│  BPM [ 93 ]  ☐ V  │ Sections  Section 2 of 3 · 93 BPM [Verse__] │ │ [>] [bar1] ▭ [bar2] [<]            │
│  [-] [+] [Reset]  │ [Add] [Remove] [×2] [÷2] [|◀ Start] [End ▶|]│ │ [Play] [Loop On] [Count-In]        │
│  Comes in a later │ This section [Click] [Count In]  <message>  │ │ <loop status line>                 │
└────────────────────────────────────────────────────────────────┘ └────────────────────────────────────┘
┌ [Rew] [Play] [FF] [0:12.3 / 3:45.0]   [══════════════════ position slider ═══════════════════════]   ┐
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **`PlayerPane`:** `player-controls` becomes a column (`flex flex-col gap-2`) of two rows.
  - **Row 1:** `flex flex-wrap items-stretch gap-2` holding `SectionsSection` (`flex-none`, its content width) and
    `LoopsSection` (`flex-1 min-w-[27rem]`). When both don't fit, Loops wraps below Sections and takes the full width.
  - **Row 2:** `PlaybackSection`, `w-full`.
  - `BpmSection` is no longer a direct child.
- **Sections pane** (`role="group" aria-label="Sections"`, as now): a row of the BPM group and the section column.
  - The BPM group keeps `role="group" aria-label="BPM"` and its content. With `nested` it has no card border or background
    of its own, only a 1 px divider on its right (`border-r`), which saves its padding and border.
  - The section column keeps its 3 rows: (1) "Sections", the status and the name box; (2) Add, Remove, ×2, ÷2, Start, End;
    (3) the "This section" group (Click, Count In) and, on the same row, the message line (`flex-1 truncate`, reserved
    height, full text as tooltip). The `absolute` message strip and the `pb-6` it needed go away.
  - At narrow widths the section column wraps below the BPM group (no sideways scroll).
- **Loops pane:** the same controls and handlers, in the existing wrapping row of three sub-groups (Loop list, Loop range,
  Loop playback). A visible `<span class="text-lg font-semibold">Loops</span>` starts the first line. Beside Sections
  (about 27 rem wide) the sub-groups fall into three lines by themselves; at full width they take one or two lines. The
  status line keeps its reserved line.
- **Playback pane:** one wrapping row: Rew, Play, FF, the time field, then the slider wrapper (`min-w-48 flex-1`).
- **Space budget at 1280x800, tree folded** (estimates; Task 29 measures). The pane's content width is about 1200 px.
  - **Widths:** Sections about 780 px (BPM group ~230 + the Add…End row ~510 + padding). That leaves about 410 px for
    Loops, which is above its 27 rem (432 px) minimum only if Sections is a little narrower. The fit is tight and must
    be measured.
  - **Heights:** Loops in three lines is about 228 px (3 × 52 + gaps + status + padding). Sections is about 168 px and is
    stretched to the row's height. Playback in one row is about 80 px.
  - **Total:** row 1 ~228 + gap 8 + Playback ~80 ≈ 316 px. Phase 2 used ≈ 412 px and scrolled 36 px, so this should not
    scroll, with about 60 px to spare.
  - If Loops wraps below Sections at 1280x800 (folded), the total becomes ~168 + 8 + ~168 (Loops in two lines at full
    width) + 8 + 80 ≈ 432 px: about 56 px of scroll, which misses the target. So the side-by-side fit is the thing to
    secure.
- **Target:** the pane scrolls **at most 14 px** (the phase-1 value) at 1280x800 with the tree folded, with Sections and
  Loops on one row; 0 is the aim.
  - If they don't fit side by side, apply in this order until they do, and report which: (1) the loop range boxes `w-24` →
    `w-20` and the loop slider `w-20` → `w-16`; (2) `min-w-[27rem]` → `min-w-[25rem]`; (3) the Sections button row
    `gap-2` → `gap-1.5` and the name box `w-48` → `w-40`.
  - If the height is still over the target: the loop status line moves to the end of the last Loops line (truncated,
    tooltip).
- **Wrapping** (allowed to scroll, never sideways, nothing overlaps):
  - **Tree shown at 1280x800** (content ~930 px): Loops wraps below Sections at full width (one or two lines). Expect
    ~430 px of controls and some scroll.
  - **1024x640, tree folded** (content ~945 px): the same wrap, and the pane scrolls more because of the height.
  - **1024x640, tree shown** (content ~620 px): the section column also wraps below the BPM group.
  - Task 30 reports the measured overflow for each of the four cases.
- **Automated measure.** `PlayerPane` logs `calliope-ui: player layout overflow=<scrollHeight - clientHeight> width=<clientWidth>`
  (integers) once 300 ms after the pane's size last changed (a `ResizeObserver` on the pane, debounced). It also logs
  ` loops_beside=<0|1>`, which is 1 when the Loops group's top equals the Sections group's top. The GUI test asserts the
  target and the side-by-side row with it.

### 18.3 The master metronome goes; `player.json` schema 5 (requirement 14)

- **Engine:** `sync_grid` sends `SetMetronome(!tempo.is_empty())`. The engine still plays the click list only when a tempo
  exists; which beats click is decided by `section_clicks` alone (Click flags, section count-ins). `Cmd::SetMetronome`
  stays (it is the "has a tempo" gate).
- **Removed:** `PlayerManager::set_metronome`, the `player_metronome` command, its ACL entry (`build.rs` list,
  `capabilities/main.json`), `SnapMetronome` and `PlayerSnapshot.metronome`, the TS wrapper and `setMetronome`, the
  frontend log `player metronome …`. No key ever toggled the master (Player keys: Space, b, e, Left, Right, l / Shift+L,
  Home / End), so no key changes.
- **Schema 5:** the top-level `metronome` object is no longer written. It is read only to migrate schema 1-4 files and is
  dropped from `extra` as before. Section 1 may carry `"count_in": true`.
  - **Why a bump:** a phase-2 build reading a phase-3 file would find no master (= Off, silence) and would drop section
    1's count-in, then write that back. With 5, older builds open it read-only (the existing mechanism).
  - Reads 1-5, writes 5; 6+ read-only.
- **Migration of schema 1-4** (owner decision Q1), after parsing the segments, using the old master (a missing object
  or key counts as Off, the old default):
  1. master `on` false → every section gets Click off **and** Count In off. Under master Off nothing clicked, section
     count-ins included, so the track stays silent.
  2. master `on` true → the section flags are kept as they are.
  3. then, master `count_in` true → section 1 gets Count In on (the Play count-in from the song start keeps working).
  4. Loops are not changed (their Count-In already gave their pre-roll).
  - **Play mid-song:** covered by the per-section rule of 18.4. Play counts in whenever the section under the playhead
    has Count In, so turning it on for a section gives the count-in there (owner, Q1).
  - **Not carried:** the old global Count In's pre-roll before loop Play. Loop Play uses the loop's own Count-In (Q4).
- **New maps** (the first typed BPM in the Player, Autodetect on a track without a tempo) start with Click on in every
  section (the existing `SectionMeta` default; owner decision Q2). Typed or added sections still copy the Click of the
  section they split (phase 2, 10.3).

### 18.4 Play and loop Play count-in (requirement 14, owner decisions Q3, Q4)

- **Play:** `play_with` computes the pre-roll as before, but the condition is "from paused or stopped **and**
  `tempo.count_in_at(position_ms)`" instead of `metronome.count_in`.
  - `count_in_at(ms) = segment_at(ms).is_some_and(|i| segments[i].meta.count_in)`. The section under the playhead:
    position at a change → the new section; before the first start → section 1.
  - The plan is `plan_at(position)`: three clicks at that section's BPM (× speed, still 1.0), the audio starts on the
    silent fourth beat, exactly the existing count-in.
  - When Play is pressed at the track end the position goes to 0 first (unchanged), and section 1 decides.
- **Count In of section k now has two effects:** the count-in while playing through its start (phase 2, unchanged, only
  for k ≥ 2) and the pre-roll when Play starts anywhere inside section k. They cannot both sound: a pre-roll starts inside
  section k, after the through-count-in span.
- **Section 1 Count In** is allowed (`set_section_flags` no longer refuses it; `parse_tempo` and `set_bpm_at` no longer
  clear it). It gives only the Play pre-roll; `section_clicks` still starts its count-in loop at k = 1 (the second
  section). The button is enabled for section 1 and its "The first section has no section before it" title goes.
- **Loop Play:** the pre-roll comes from the loop's Count-In only (`if l.count_in { plan_at(start) }`); the old
  `metronome.count_in ||` term goes. The repeat count-in (each pass with Count-In) is unchanged.
- **Jumps and seeks** (Start / End, Prev / Next, Rew / FF, the slider) never play a pre-roll themselves; a later Play from
  paused applies the rule above (as in phase 2).
- **Log:** `calliope: player count-in clicks=3 beat_ms=<ms>` is unchanged; Play gains ` section=<i>` on that line (Rust
  `log_count_in` takes an optional index), so the GUI test can tell which section counted in.

### 18.5 Adding a loop: the default end (requirement 16)

**Today:** the Loops pane's "+" button (`aria-label="Add loop"`, no key) sends `player_loop` `{kind: 'add'}` →
`LoopAction::Add` → `PlayerState::add(pos_ms)`: a loop starting at the playhead (`boundary_at`: the nearest tick in BPM
mode, exact ms in timestamp mode), **no end**, `repeat: true`, `count_in: false`, made current. The end is set later with
"<" / `e` or by typing bar2. Tempo-section Add (Sections pane, `a` on the waveform) is separate and unchanged.

**New:** `PlayerState::add(position_ms, track_end_ms)`:
1. Refused at 99 loops, as today ("At most 99 loops"), before anything else.
2. `start = boundary_at(position_ms)`.
3. `end = default_end(start, track_end_ms)`:
   - **BPM mode:** `t0` = the start tick.
     - `next = tempo.next_section_tick(t0)`: the smallest segment tick base greater than `t0` (`bases()`), so the start
       tick's own section decides, even when the playhead was a fraction of a tick before a change.
     - If `next` exists and `ms_at(next) <= track_end_ms` → `Tick { next }`. For detected and placed changes
       `ms_at(base)` is exactly the section's `start_ms`; for a typed change it is within half a tick (the tick rule).
     - Else (the last section) → the track end: `t = tick_near(track_end_ms)`, minus 1 if `ms_at(t) > track_end_ms`, so
       the end never lies past the audio and passes the existing "within the track" check of a typed end.
   - **Timestamp mode** (no tempo, so no sections): `Ms { track_end_ms }` (owner decision Q6).
   - If the end would not be after the start (the start is within a tick of the track end) → no end, as today.
4. `repeat: true`, `count_in: false` (unchanged defaults; owner decision Q5). The loop is complete at once, so
   loop Play and Loop On are enabled.

**Consequences:**
- The new loop ends where the next section starts. With Loop On (the default), normal playback that reaches it now
  jumps back to the loop start, as with any complete repeating loop; pressing `e` earlier still shortens it.
- **Interplay with phase 2:** a repeating loop that ends exactly at a section with Count In skips that count-in at its end
  (10.6); that is the "practise one section" case, and nothing new is needed.
- `sync_loop` already re-sends the loop and, when the repeating loop changed, the grid.
- `PlayerManager::loop_action` passes `s.ms_of(s.pcm.frames)` as `track_end_ms`.
- **The frontend is unchanged** except the log line, which becomes
  `player loop <name> index=… count=… complete=… end=<end_ms rounded|none>`.

### 18.6 Autodetect keeps the play settings (owner decision Q7)

Without the master, a re-detection in the Editor would reset every section to Click on, no count-in, and so lose a
"quiet" track or its Play count-in.
- `TempoMap::with_play_settings_of(&self, old) -> TempoMap`: when `old` is not empty, every new section gets `old`'s
  section-1 Click, and the new section 1 gets `old`'s section-1 Count In. Names are still lost (phase-2 Q8), and the other
  sections get no count-in.
- `beat_detect.rs` returns `Ok(new.with_play_settings_of(old))` from its `replace` closure.
- No Editor view change. The Editor's move of a detected change keeps the segment, as before.

### 18.7 Tests that used the master

| Where | Uses | Becomes |
|---|---|---|
| `player_tests.rs` (17 calls) | `set_metronome(true, _)` to hear clicks | maps from `set_bpm` already click; fixtures read from schema ≤ 4 files get `"metronome":{"on":true,"count_in":false}` added, so the migration keeps their flags |
| | `set_metronome(_, true)` for a pre-roll | `SectionEdit::Flags` with `count_in: true` on the section at the playhead |
| | master-only tests (`metronome_off_leaves_the_source_untouched`, `master_off_silences_…`, `metronome_and_speed_are_refused_without_a_tempo`, `removing_the_tempo_clears_…`) | rewritten for Click off / section Count In; the "refused without a tempo" part stays for speed and for `section-flags` on an empty map |
| `player_ipc_tests.rs` | `set_metronome` + `snap.metronome` | a `section-flags` round trip |
| `acceptance_player_backend.rs` (35) | as above, the `metronome_on()` helper, `d["metronome"]`, `s.metronome` | the same rules; `metronome_on()` is removed and its fixtures carry the master On instead; `ac22_plus_creates_loop_without_end…` and `ac28` follow 18.5 (time mode: end = track end); `ac13` becomes "a hostile schema-1 file with the master on at BPM 0 makes no click and no count-in" (still true) |
| vitest: `ControlBar.test.ts`, `acceptance_player.test.ts`, `ipc-player.test.ts`, `player-state.test.ts`, `PlayerView.test.ts` | the "Metronome" group, `player_metronome`, `metronome` in snapshot fixtures | removed / replaced by the section toggles |
| GUI: `gui_player_e2e.rs` (`METRO`, `METRO_COUNT_IN` in `tempo_metronome_count_in`, `player_tempo_sections`, `player_section_metronome`, `visual_review`), `gui_player_acceptance_e2e.rs` (`bpm_zero_has_no_clicks…`, `loop_without_end…`) | the master buttons, `player metronome on=…` lines, "+ makes a loop without an end" | `SEC_CLICK` / `SEC_COUNT_IN` and the `player section flags …` lines; the loop test becomes "+ gives the default end; < at the start is refused and the end stays" |

The tests that rely on the earlier spec's behaviour (`gui-play-backing-track` requirements 10 and 15: Metronome On/Off and
a global Count In) are changed on purpose: requirement 14 of this spec replaces them.

## 19. Tasks (phase 3)

Same rules as sections 3 and 11: Rust tasks end with `cargo clippy` clean for what they build, frontend tasks with
`npm run check` clean (both in `src/calliope-gui/`). Never commit in a routine task.

### Task 23: Tempo map: section-1 Count In, `count_in_at`, `next_section_tick`, `with_play_settings_of`
- **files:** `src/calliope-gui/src/tempo_map.rs`
- **does:**
  - `set_section_flags`: drop the index-0 count-in refusal.
  - `set_bpm_at`: when the first segment is removed, the new first keeps its `count_in`.
  - `pub fn count_in_at(&self, ms: f64) -> bool` (18.4).
  - `pub fn next_section_tick(&self, tick: i64) -> Option<i64>`: the smallest of `bases()` greater than `tick`.
  - `pub fn with_play_settings_of(&self, old: &TempoMap) -> TempoMap` (18.6).
  - Update the doc comment of `SectionMeta.count_in` ("on the first segment: only the Play count-in").
- **tests** (in `tempo_map.rs`):
  - section 0 `count_in: true` is accepted; the phase-2 refusal test is replaced;
  - `set_bpm_at` BPM 0 on the first segment keeps the new first's count-in;
  - `count_in_at` before the first start, at a change (the new section), inside, on an empty map (false);
  - `next_section_tick` on `[(500,120),(8500,93 detected)]`: from tick 0 → the base of 8500, and `ms_at` of it is
    8500 exactly; from that base → None; on a typed change the result's `ms_at` is within half a tick of `start_ms`;
  - `with_play_settings_of`: old section 1 Click off + Count In on → all new Click off, new section 1 Count In on, others
    off, no names; old empty → the new map unchanged.
- **done when:** `cargo test -p calliope-gui --lib tempo_map` passes; clippy clean.
- **routine:** no

### Task 24: Remove the `player_metronome` command
- **files:** `src/calliope-gui/build.rs`, `src/calliope-gui/src/gui.rs`, `src/calliope-gui/src/player_ipc.rs`,
  `src/calliope-gui/capabilities/main.json`
- **does:** exact deletions, nothing else:
  1. `build.rs`: delete the line `    "player_metronome",`.
  2. `src/gui.rs`: delete the line `            player_ipc::player_metronome,`.
  3. `src/player_ipc.rs`: delete these lines and the blank line after them:
     ```rust
     #[tauri::command]
     pub(crate) async fn player_metronome(app: tauri::AppHandle, id: String, on: bool, count_in: bool) -> Result<PlayerSnapshot, String> {
         blocking(move || app.state::<Arc<PlayerHost>>().with(|m| m.set_metronome(&id, on, count_in, Instant::now()))).await
     }
     ```
  4. `capabilities/main.json`: delete the line `    "allow-player-metronome",`.
  5. If the generated file `src/calliope-gui/permissions/autogenerated/player_metronome.toml` exists, delete it (it is
     not tracked by git).
- **done when:** `cd src/calliope-gui && cargo test -p calliope-gui --lib player_ipc` passes and
  `git grep -n player_metronome -- build.rs src/gui.rs src/player_ipc.rs capabilities` prints nothing. Run it once, then
  stop. Do not commit.
- **routine:** yes

### Task 25: Backend: schema 5, migration, no master, Play count-in per section, default loop end, Autodetect carry
- **files:** `src/calliope-gui/src/player_state.rs`, `src/calliope-gui/src/tempo_store.rs`, `src/calliope-gui/src/player.rs`,
  `src/calliope-gui/src/beat_detect.rs`, `src/calliope-gui/src/player_tests.rs`, `src/calliope-gui/src/player_ipc_tests.rs`,
  `src/calliope-gui/src/beat_detect_tests.rs` (one new test, and `assert_eq!(r.player_json().unwrap()["schema_version"], 4);`
  → `5`)
- **does:**
  - **`player_state.rs`:**
    - `SCHEMA = 5`; remove `Metronome` and the field;
    - `from_value` reads the old master only when `schema_version <= 4` and applies 18.3;
    - `parse_tempo` keeps section 1's `count_in`;
    - `add(position_ms, track_end_ms)` with the default end (18.5);
    - "written as 4" asserts → 5, the "newer" fixtures `5` → `6` in this file and in `tempo_store.rs`.
  - **`player.rs`:**
    - remove `set_metronome` and `SnapMetronome`;
    - `sync_grid` sends `SetMetronome(!tempo.is_empty())`;
    - `play_with` uses `count_in_at` (18.4); loop Play uses the loop's Count-In only;
    - `loop_action` passes the duration to `add`;
    - `log_count_in` appends ` section=<i>` for Play.
  - **`beat_detect.rs`:** 18.6.
  - **Tests:** migrate the master uses in `player_tests.rs` and `player_ipc_tests.rs` as in 18.7.
- **new tests:**
  - **`player_state.rs`:**
    - v4 master Off with section flags (Click on, Count In on section 2) → all Click off, all Count In off;
    - master On → the flags are kept;
    - master `count_in` true → section 1 Count In on (with master Off too);
    - a file without a `metronome` object counts as Off;
    - the written file is schema 5 and has no top-level `metronome`;
    - a v5 file with section-1 `count_in` round-trips; a v5 file with a stray top-level `metronome` is not migrated
      (the flags are kept) and the key is not written back;
    - `add`, BPM mode: the end is the next section's base (detected map: `end_ms` = 8500 exactly), and the last section
      gives the track end, never past it;
    - `add`, timestamp mode → `Ms { track_end_ms }`;
    - `add` at the track end → no end;
    - the 100th loop is refused.
  - **`player_tests.rs`** (ManualBackend):
    - Play from paused inside a section with Count In → three clicks at that section's beat, then the audio from the
      position; inside a section without → the audio at once (first non-silent frame within the 3 ms fade);
    - Play at 0 with section 1 Count In → the pre-roll at section 1's tempo;
    - loop Play with the loop's Count-In off over a section with Count In → no pre-roll;
    - the engine metronome flag is on whenever there is a tempo; a section with Click off is silent;
    - "+" while playing makes a complete repeating loop and playback wraps at the next section start.
  - **`beat_detect_tests.rs`:** a detection over a schema-4 `player.json` with master Off and Count In on → the written
    schema-5 map has every section Click off and section 1 Count In on.
- **done when:** `cargo test -p calliope-gui --lib` passes and `cargo clippy -p calliope-gui --lib --tests -- -D warnings` is
  clean. The integration tests compile again after Tasks 26-27.
- **routine:** no

### Task 26: Schema-number literals in the other tests
- **files:** `src/calliope-gui/tests/acceptance_beat_autodetect.rs`, `src/calliope-gui/tests/gui_editor_e2e.rs`,
  `src/calliope-gui/tests/gui_player_e2e.rs`
- **does:** exact edits. Change only the lines listed; line numbers are from `ce958d3` and may be a few lines off, so find
  each by its text:
  1. "written as 4" becomes "written as 5": replace `4` with `5` in each of these lines:
     - `acceptance_beat_autodetect.rs:460` and `:713`: `assert_eq!(v["schema_version"], 4);`
     - `gui_editor_e2e.rs:361`, `gui_player_e2e.rs:363`, `:432` and `:538`: `assert_eq!(v["schema_version"], 4, "{v}");`
  2. The "newer schema" fixture: replace `"schema_version":5` with `"schema_version":6` in
     `acceptance_beat_autodetect.rs:611` (`let newer = r#"{"schema_version":5,"tempo":…`).
  3. Leave every other `schema_version` alone: fixtures that are read as schema 1-4 test the migration.
     `beat_detect_tests.rs` and `acceptance_player_backend.rs` are done in Tasks 25 and 27.
- **done when:** `cd src/calliope-gui && cargo test -p calliope-gui --test acceptance_beat_autodetect && cargo test -p
  calliope-gui --features e2e-hooks --test gui_editor_e2e --test gui_player_e2e --no-run` passes. Run it once, then stop.
  Do not commit.
- **routine:** yes

### Task 27: Backend acceptance tests: the master removed, TS19-TS24
- **files:** `src/calliope-gui/tests/acceptance_player_backend.rs`
- **does:**
  - Migrate the master uses as in 18.7: "written as 4" → 5; newer fixtures 5 → 6; `ac22`, `ac28` and `ac13` as described
    there.
  - A new section "phase 3 (gui-player-tempo-sections)" with these tests, reusing `rig_with`, `clicks`, `onsets` and the
    TS fixture:
    (the TS fixture is schema 2 without a master, so it opens with every Click off (18.3): set the flags each test needs)
    - `ts19_play_counts_in_at_the_section_under_the_playhead`:
      - section 2 (93 BPM from 8500) Count In; seek 10000; Play → onsets at 0, 645.16 and 1290.32 ms after Play (±1
        frame), then the source from 10000 at 1935.48 ms;
      - seek 2000 (section 1, Count In off) → no click before the audio, and the audio starts at once;
      - section 1 Count In, seek 0 → the pre-roll at 120 BPM (500 ms apart).
    - `ts20_no_master_clicks_only_where_click_is_on`:
      - the snapshot JSON has no `metronome` key;
      - section 1 Click off and section 2 Click on → no onset before 8500, onsets at 93 BPM after;
      - all Click off → silence;
      - a section Count In still plays over a silent previous section (phase 2 Q2 unchanged).
    - `ts21_new_loop_ends_at_the_next_section_or_the_track_end`:
      - seek 3000, "+" → `end_ms` 8500 (detected change, exact);
      - seek 12000, "+" → the track end (≤ duration, within one tick);
      - a typed-change fixture → within half a tick of its `start_ms`;
      - timestamp mode → `end` = `{ms: duration}`;
      - at 99 loops → refused.
    - `ts22_old_files_migrate_to_schema_5`:
      - v1-v4 with master Off → every section silent, no section count-in;
      - v4 master On with mixed flags → the same clicks as phase 2;
      - master Count In → section 1 Count In;
      - written as 5 without a top-level `metronome`;
      - a schema-6 file is never overwritten by flags, name, jump or "+".
    - `ts23_loop_play_counts_in_only_with_the_loops_count_in`: loop Play over a section with Count In, the loop's
      Count-In off → no pre-roll; on → the pre-roll at the BPM at the loop start, and on each pass as before.
    - `ts24_autodetect_keeps_the_play_settings`: the Editor's `TempoStore::replace` with `with_play_settings_of` over a
      file with section 1 Click off and Count In on → the same in the new map.
- **done when:** `cargo test -p calliope-gui --test acceptance_player_backend` passes; `cargo test --workspace` passes;
  `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- **routine:** no

### Task 28: Frontend: no master metronome, section-1 Count In, loop log end
- **files:**
  - `src/calliope-gui/src/ui/lib/ipc.ts`, `src/calliope-gui/src/ui/lib/ipc-player.test.ts`;
  - `src/calliope-gui/src/ui/lib/player-state.svelte.ts`, `src/calliope-gui/src/ui/lib/player-state.test.ts`;
  - `src/calliope-gui/src/ui/components/player/SectionsSection.svelte`;
  - `src/calliope-gui/src/ui/components/player/ControlBar.test.ts`, `src/calliope-gui/src/ui/acceptance_player.test.ts`,
    `src/calliope-gui/src/ui/views/PlayerView.test.ts`.
- **does:**
  - Remove `PlayerSnapshot.metronome`, `playerMetronome`, `setMetronome`, and the "Metronome" sub-group with its
    `mOn`/`mCountIn`/`usable` code.
  - Count In is enabled on section 1 (only "Type a BPM first" disables it).
  - The loop log line gains ` end=<ms|none>` (18.5).
  - Remove `metronome: {…}` from the snapshot fixtures of the test files listed.
- **tests:**
  - **`ControlBar.test.ts`:** no "On/Off" button and no "Metronome" group anywhere; section 1 Count In enabled and sends
    `section-flags index=0 count_in=true`; the old master tests are replaced by the section toggles.
  - **`acceptance_player.test.ts`:** the group list is `BPM`, `Sections`, `Playback`, `Loops` (no `Metronome`); the BPM-0
    test checks the section Click / Count In are disabled with "Type a BPM first".
  - **`ipc-player.test.ts`:** the `playerMetronome` lines are removed and the command list no longer has
    `player_metronome`.
  - **`player-state.test.ts`:** the loop log line has `end=`.
- **done when:** `cd src/calliope-gui && npx vitest run src/ui/` passes; `npm run check` is clean;
  `git grep -n "player_metronome\|playerMetronome\|setMetronome" -- src/ui` prints nothing.
- **routine:** no

### Task 29: Player layout: Sections (BPM inside) and Loops side by side, Playback at the bottom
- **files:**
  - `src/calliope-gui/src/ui/components/player/PlayerPane.svelte`, `SectionsSection.svelte`, `BpmSection.svelte`,
    `LoopsSection.svelte`, `PlaybackSection.svelte`;
  - `src/calliope-gui/src/ui/components/player/ControlBar.test.ts`, `src/calliope-gui/src/ui/acceptance_player.test.ts`.
- **does:** 18.2, including the `player layout overflow=… width=… loops_beside=…` log line. Measure at 1280x800 with the
  tree folded and apply the fallbacks of 18.2 until Loops sits beside Sections and the overflow is ≤ 14 px.
- **tests:**
  - **`acceptance_player.test.ts`** (DOM order):
    - `player-controls` has two rows: the first holds the groups Sections and Loops in that order, the second the group
      Playback;
    - the BPM group is inside Sections and comes before `section-status` in document order;
    - the Loops group shows the text "Loops";
    - the Playback group holds Rew, Play, FF, the time field and the Position slider.
  - **`ControlBar.test.ts`:** the message line keeps its height when it fills (no layout shift: the same `offsetHeight`
    class/min-height before and after an error).
  - **Visual check** (implementer, not automated here): screenshots at 1280x800 and 1024x640, tree folded and shown,
    both themes, through `CALLIOPE_GUI_SHOTS=1 npm run test:gui -- visual_review` (positions are fixed in Task 30, so a
    quick manual `gui-shot` on `:1` after `npm run dev:app` is enough here).
- **done when:** `npx vitest run src/ui/components/player/ src/ui/acceptance_player.test.ts src/ui/views/` passes;
  `npm run check` is clean; the screenshots show the two rows and nothing overlaps; the measured `overflow` and
  `loops_beside` at 1280x800 with the tree folded are reported.
- **routine:** no

### Task 30: GUI end-to-end, re-measured positions, layout target, docs
- **files:** `src/calliope-gui/tests/gui_player_e2e.rs`, `src/calliope-gui/tests/gui_player_acceptance_e2e.rs`,
  `docs/ui.md`
- **does:**
  - **Positions:** re-measure every fixed position in both files at 1280x800 with the tree folded, from screenshots
    (`BPM`, `PLAY`, `TIME`, `LOOP_*`, `MARK_*`, `SEC_*`). Delete `METRO` and `METRO_COUNT_IN`.
  - **Existing tests** (18.7):
    - `tempo_metronome_count_in`: Click via `SEC_CLICK`, the pre-roll via `SEC_COUNT_IN` on section 1;
    - `player_section_metronome`: its fixture is written as schema 5, the master step removed;
    - `player_tempo_sections` and `visual_review` use the section toggles;
    - `bpm_zero_has_no_clicks…`: the section toggles are disabled at BPM 0, and clicking them logs nothing;
    - `loop_without_end…` becomes `loop_add_ends_at_the_next_section_and_a_bad_end_is_refused`.
  - **New test `player_phase3`** (capture as in `tempo_metronome_count_in`; TS fixture written as schema 5 with section 2
    Count In on and section 1 Click off):
    1. pick the track; at 1280x800 with the tree folded, wait for `player layout overflow=`, assert `overflow` ≤ 14 and
       `loops_beside=1`; screenshot `player-p3-layout`;
    2. seek 10.0 s, Space → `player count-in clicks=3 beat_ms=645.16 section=1`; onsets 645 ms apart (±5 ms), then the
       tone; Pause;
    3. seek 2.0 s, Space → no `player count-in` line; the tone starts within 100 ms and no onset before 8.5 s (Click off
       in section 1); Pause;
    4. seek 3.0 s, `LOOP_ADD` → `player loop add … complete=true end=8500`; seek 12.0 s, `LOOP_ADD` →
       `end=<duration ±1>`;
    5. after 0.6 s, `player.json` is schema 5, has no top-level `metronome`, and `loops[0].end` is the tick of 8500.
  - **`visual_review`:** the new layout in both themes, both sizes, tree folded and shown. Report `overflow` and
    `loops_beside` for each (expected: beside only at 1280x800 folded; elsewhere Loops wraps below Sections, never
    sideways) and judge:
    - BPM and section controls are readable at 1-2 m;
    - the Loops title is visible;
    - Sections and Loops share row 1 at 1280x800 folded, and Playback is the bottom row with a usable slider;
    - nothing moves when the section message or the loop status fills.
  - **`docs/ui.md`:** replace the phase-2 layout paragraphs (the Sections group, the toggles, Layout, the Metronome
    group mentions) with the phase-3 layout and rules: no master, section-1 Count In, the Play pre-roll per section, the
    default loop end, the measured overflow.
- **done when:**
  - `cd src/calliope-gui && npm run test:gui -- player_phase3` passes (private display);
  - the screenshots were reviewed and the measured overflow values are reported;
  - `npm test` and `npm run test:gui` pass.
- **routine:** no

### Task 31: README Player usage
- **files:** `src/calliope-gui/README.md`
- **does:** exact text replacements in the "## Player" section, nothing else (follows the owner decisions of 24):
  1. Replace the whole bullet that starts `* BPM: type a tempo (20 to 300; 0 clears it) to place it at the playback position. Metronome:`
     (3 lines) with:
     ```text
     * Under the waveform: the Sections pane (with the BPM field on its left) and the Loops pane side by side (Loops
       goes below Sections in a narrow window), and Playback at the bottom. BPM: type a tempo (20 to 300; 0 clears
       it) to place it at the playback position. The bar fields of the loops need a BPM; without one they show times.
     ```
  2. In the bullet that starts `* Per section (Sections group, for the selected section):`, replace the text from
     `and Count In (three clicks` up to the end of that bullet (`also the section count-ins.`) with:
     ```text
       and Count In (three clicks at this section's tempo just before it, replacing the end of the section
       before, and skipped where a repeating loop ends at the section; Play pressed anywhere in a section with
       Count In counts in at its tempo, also in the first section), and Start / End (jump to the section's
       first beat / the start of its last bar; playing continues without a pause). There is no global
       metronome switch: a section clicks only when its Click is on.
     ```
  3. Replace the bullet that starts `* Loops: "+" starts a loop at the position (it has no end until you press "<")` (3
     lines) with:
     ```text
     * Loops: "+" starts a loop at the position and ends it where the next tempo section starts (or at the end of
       the track), ">" and "<" set the start and end at the position, Prev/Next select, "-" removes. The loop Play
       button plays the current loop; Loop On repeats it, Count-In counts in before every pass.
     ```
  4. In the bullet that starts `* Everything is saved by itself`, replace `(source, tempo, metronome,` with
     `(source, tempo sections with their settings,`.
- **done when:** `git diff --stat -- src/calliope-gui/README.md` shows only that file and
  `grep -c "Metronome group\|no end until" src/calliope-gui/README.md` prints `0`. Run it once, then stop. Do not commit.
- **routine:** yes

**Order (phase 3):**
- Rust: 23 → 24 → 25 → 26 → 27.
- Frontend: 28 → 29 (28 may start after 25).
- Then 30. 31 any time.

## 20. Test strategy (phase 3 additions)

- **Pure Rust:**
  - `tempo_map`: `count_in_at`, `next_section_tick`, `with_play_settings_of`, section-1 Count In (Task 23);
  - `player_state`: the schema-5 migration and the default loop end in both modes and at the edges (Task 25).
- **Player session** (`player_tests`, ManualBackend, injected `Instant`s): the Play pre-roll per section, loop Play
  pre-roll, the metronome gate, "+" while playing (Task 25).
- **Audio behaviour** (`acceptance_player_backend`, ManualBackend): onsets at exact frames for TS19, TS20 and TS23; loop
  ends for TS21; old files for TS22; Autodetect carry for TS24 (Task 27). `CpalBackend` is never constructed in tests.
- **Frontend:** vitest + mockIPC for the removed master, section-1 Count In, the DOM order of the panes (Tasks 28-29);
  `svelte-check`.
- **GUI:** `player_phase3` on a private display with `CALLIOPE_E2E_AUDIO=capture:` onset analysis, the
  `player layout overflow=` log for the scroll target, screenshots and the visual review (Task 30). Visual review is never
  routine.
- **Not involved:** calliope-stems (except the existing detection tests), the network, archserver, other hosts, any real
  gear.
- **Whole suite (unchanged):** `cd src/calliope-gui && npm test`, then `npm run test:gui`.

## 21. Deployment (phase 3)

Laptop only, as in sections 5 and 13: `cd src/calliope-gui && npm run build:app` and install. Nothing changes on
archserver.

**Note:** this build writes `player.json` as schema 5 and migrates older files on their first save (18.3). The phase-2
build (schema 4) opens schema-5 files read-only: it plays with defaults and never overwrites them. Don't mix builds on the
same repository.

## 22. Manual checks (phase 3, owner: laptop with real audio)

- **M14: Layout.** At 1280x800 with the tree folded (and at your usual size):
  1. under the waveform you see Sections (BPM on its left) and Loops (with its title) side by side, and Playback at the
     bottom;
  2. nothing needs scrolling at 1280x800 with the tree folded;
  3. it is readable at 1-2 m, and Play is easy to find at the bottom.
- **M15: Your existing tracks.** Open a track you used with phase 2.
  1. With the old Metronome Off it is silent; with it On, the sections click as before.
  2. If the old Count In was on, Play from the start still counts in; elsewhere, turn on a section's Count In and Play
     inside it counts in.
  3. Loop Play counts in only with the loop's own Count-In.
- **M16: Play count-in per section.**
  1. Turn Count In on for a middle section, pause inside it and press Play: three clicks at that section's tempo, then
     the music.
  2. Pause in a section without Count In: Play starts at once.
- **M17: Click only where wanted.** Turn Click off for the intro and on for the verse: the intro is silent and the verse
  clicks. No global On/Off is left.
- **M18: Default loop end.**
  1. Pause inside a section and press "+": the loop ends exactly where the next section starts (the waveform marks and
     bar2 agree). In the last section it ends at the track end.
  2. Loop On repeats it; pressing `e` earlier while playing still shortens it.
- **M19: Re-detect.** Run Autodetect again in the Editor on a track whose sections are silent and whose first section has
  Count In: back in the Player it is still silent and Play still counts in.

## 23. Acceptance mapping (phase 3)

| # | Criterion | Spec | Tasks | Tests |
|---|---|---|---|---|
| TS18 | Given the Player view, then under the waveform come the Sections pane (BPM group, then the section controls in 3 rows) and the Loops pane labelled "Loops" side by side on one full-width row, and the playback pane in a full-width row at the bottom (owner Q8: two rows instead of the spec's three panes stacked) | req 12, 13, 15 | 29, 30 | acceptance_player.test (DOM rows and order), gui `player_phase3` step 1 (overflow ≤ 14 px, `loops_beside=1`), visual_review screenshots |
| TS19 | Given the section under the playhead has Count In on, when the user presses Play, then the count-in plays at that section's tempo; with Count In off, playback starts at once | req 14 | 23, 25, 27, 30 | tempo_map (`count_in_at`), player_tests, `ts19_*`, gui step 2-3 |
| TS20 | Given the Player view, then there is no global Metronome On/Off or global Count In button, and the metronome clicks only in sections whose Click is on | req 14 | 24, 25, 27, 28, 30 | `ts20_*`, ControlBar.test, acceptance_player.test, gui step 3 |
| TS21 | Given the playhead inside a tempo section that has a next section, when the user adds a loop there, then the loop starts at the playhead and ends at the next section's start; in the last section it ends at the track end | req 16 | 23, 25, 27, 28, 30 | tempo_map (`next_section_tick`), player_state (`add`), `ts21_*`, gui step 4-5 |
| TS22 (added) | Given a `player.json` of schema 1-4, then it plays as before (master Off → silent, master On → the section flags, master Count In → section 1 Count In) and is written as 5 without a global metronome; a schema-6 file is never overwritten | 18.3 | 25, 26, 27 | player_state, `ts22_*`, beat_detect_tests |
| TS23 (added) | Given a loop, then loop Play counts in only when the loop's Count-In is on; loops stay separate from sections (own pane, own "+") | req 14, 15 | 25, 27, 29 | player_tests, `ts23_*`, acceptance_player.test |
| TS24 (added) | Given Autodetect in the Editor, then the new sections keep section 1's Click and Count In | 18.6 | 23, 25, 27 | tempo_map, beat_detect_tests, `ts24_*` |

## 24. Owner decisions (phase 3; 2026-10-18)

The owner accepted Q1-Q7 as recommended (Q1 with a clarification) and changed the layout (Q8). The design above follows these answers.

1. **Migrating the old global metronome (schema 1-4 files): (a).**
   - Master Off → every section's Click and Count In off (silent, as before).
   - Master On → the section flags are kept.
   - Master Count In → section 1's Count In.
   - **Clarification:** pressing Play mid-section counts in when the section under the playhead has Count In (the Q3
     rule), so Play mid-song is covered per section. Only the old global count-in before loop Play is not carried; loop
     Play uses the loop's own Count-In (Q4).
2. **Click in the sections of a new tempo map: (a) on** (today's section default).
3. **Count In of a section has both effects: (a).** The count-in when playing through the section start, and the Play
   pre-roll when Play starts anywhere in the section. Section 1's Count In is available and means the Play pre-roll only.
4. **Loop Play count-in: (a)** only the loop's Count-In.
5. **A new loop with its default end repeats at once: (a)** Loop On, as every complete loop; `e` earlier still shortens
   it.
6. **Timestamp mode: (a)** "+" ends the loop at the track end.
7. **Autodetect keeps section 1's Click (for every new section) and its Count In (for the new section 1): (a).** Data
   only, the Editor view is unchanged.
8. **Layout: changed by the owner.** Two full-width rows instead of three stacked panes:
   - row 1: the Sections pane (3 rows inside, BPM group on its left, no card border of its own; the message line in row
     3) and the Loops pane side by side;
   - row 2: the Playback pane at the very bottom, in one row.

   At 1280x800 with the tree folded the pane scrolls at most 14 px (expected 0: ~316 px of controls against phase 2's
   ~412). Elsewhere (tree shown, 1024x640) Loops may wrap below Sections; there is never a sideways scroll. See 18.2.

**Addendum (2026-10-18, after task 29).** Measured at 1280x800 with the tree folded, the Player pane is 965 px wide
(the left navigation takes ~310 px), not ~1200 px, so Sections (~822 px) and Loops (~430 px) could not sit side by
side (`loops_beside=0`, overflow 0). The owner chose a **compact redesign** so they fit side by side at that size with
the navigation shown:
- the BPM input moves into the Sections title row; the disabled -/+/Reset placeholders and "Comes in a later update"
  are hidden until that feature exists;
- the section buttons get compact (icon-sized) widths; Sections keeps 3 rows;
- Loops stacks its controls in 3 short lines, about 430 px wide.
Target unchanged: `loops_beside=1` and overflow at most 14 px at 1280x800 with the tree folded. This is done as a
rework of task 29 ("task 29b") before task 30.
