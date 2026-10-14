# Plan: Player tempo sections editing

Revision 1. Base `0bd82c3`, branch `spec/gui-player-tempo-sections`. Builds on:
- `specs/gui-play-backing-track.plan.md`: tempo map 2.3, `player.json` 2.5, engine 2.7, BPM field 2.8, IPC 2.9,
  waveform 2.10;
- `specs/stems-beat-autodetect.plan.md`: detected changes (`detected_ms`, 1/64-beat steps, ±256 steps, stretched
  last beat) 2.3, `TempoStore` and the Player release 2.2, the shared `WaveformView` 2.9-2.10, owner decisions 8.

The design below follows the recommended answers of section 8. Where an answer would change a task, the question
says which.

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
refuses with "The tempo sections changed; try again" when it doesn't match exactly. JSON numbers round-trip `f64`
exactly, so equality is safe. A quick double Remove therefore never removes the wrong change.

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
