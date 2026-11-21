# Plan: Player transport: to start, click to seek, metronome off by default

Spec: `specs/gui-player-transport.md`. Architecture: `docs/architecture.md`.

**Open questions: none.** Decisions the spec leaves to the team are listed in §2.6. The owner can overrule
any of them.

## 1. Summary

Every section created from now on starts with its Click (metronome) off. That covers the first BPM on a
track with no tempo, Autodetect, the BPM box splitting a section, and Add. Sections already saved keep
their setting. The `player.json` format does not change: a missing key still means on and `false` means
off, so there is no schema bump and the previous build reads and writes the new files the same way. The
Playback group gets a "To start" button before Rew. A click on the Player's waveform (not on a marker or
a label) now also moves the playhead. With a tempo it goes to the nearest beat of the tempo grid; with no
tempo it goes to the exact clicked position. Both jumps use a new instant-jump command that keeps
playing and stays inside the song.

## 2. Design

### 2.1 Metronome default (requirements 1, 2)

- **The file format does not change.** `SectionMeta.metronome` keeps
  `#[serde(default = "yes", skip_serializing_if = "is_true")]`, and `player_state::parse_tempo` keeps
  `metronome: s.get("metronome") != Some(&Value::Bool(false))`. A saved "on" section (no key) still reads
  as on, so requirement 2 holds as it does today. A new section is off in memory and is written as
  `"metronome": false`. Turning it on removes the key. Reading back gives the same values.
  - **Why no schema bump:** the meaning of every stored value stays the same. Only the value given to new
    sections changes. The previous build (schema 7) reads, plays and writes these files exactly as this
    build does. A bump would make the previous build treat every touched file as read-only, for no
    gain.
  - **Rejected: always writing the key.** It would also work for the previous build, but it changes every
    file on its next save and breaks existing "on = no key" assertions in tests. It gains nothing.
  - **Frontend:** `section-meta.ts` (`seg.metronome !== false`) and the snapshot serialisation stay as they
    are. A new section reaches the frontend as `"metronome": false`.
- **`SectionMeta::default()` stays "on".** It is the meaning of a missing key and the base of many test
  helpers. Add `SectionMeta::new_section()` (name "", `metronome: false`, `count_in: false`) and use it
  in every place that creates a section. Its doc comment says that new sections must use it.
  - `TempoMap::set_bpm_at`: the empty-map branch (the first BPM on a track) and the insert branch (the
    BPM box splitting a section). The insert branch stops copying the split section's flag.
  - `TempoMap::add_change` (Add): stops copying the left section's flag.
  - `tempo_detect::detect`: the first segment and every detected segment.
  - `TempoMap::with_play_settings_of` (Autodetect on a track that had a tempo): every section gets
    `metronome = false`. The old first section's Count In is still carried to the new first section
    (unchanged, §2.6 d). The doc comment is updated.
  - Unchanged: Remove (drops the joined section's flags), the first-beat move (makes no new section),
    `set_section_bpm` / `scale_section` / the section BPM detection (they change an existing section's BPM
    and keep its flags; the detection is refused without a tempo, so it never makes a section, §2.6 e),
    and the schema 1-4 master migration.
- **Ripple:** tests that relied on new sections clicking (Rust unit and acceptance tests, GUI tests)
  must now switch Click on explicitly. This happens through `set_section_flags` / `SectionEdit::Flags`,
  through a fixture `player.json` without the key, or by pressing Click in GUI tests. Tests that checked
  the old copy rules (Add copies the flag, Autodetect copies the Click) are changed to the new rule. The
  acceptance tests of earlier features may be edited for this: the owner changed that behaviour.

### 2.2 Nearest beat (requirement 5): one source of truth in Rust

`TempoMap::nearest_beat_ms(&self, ms: f64, end_ms: f64) -> Option<f64>`:

- It returns `None` on an empty map, or when `ms` or `end_ms` is not finite.
- It takes the beats from `self.beats_in(lo, hi)`, the same grid `metronome::section_clicks` clicks on:
  whole-beat ticks, the stretched last beat before detected or placed changes, and the first tempo
  extended backwards. The window is `lo = max(0, ms - W)` and `hi = min(end_ms, ms + W)` with
  `W = 6000.0` ms. One beat at 20 BPM is 3 s, and a stretched beat is at most 1.5 beats, so the window
  always holds the beats on both sides.
- It returns the beat with the smallest `|beat - ms|`. On a tie the earlier beat wins (strict `<` while
  scanning in order). With no beat in the window it returns `None`.
- **Why Rust:** the frontend only has the segments and would have to repeat the tick and stretch rules
  (`waveform-geometry.ts` only draws the section starts). Computing the beat next to `beats_in` keeps the
  snapped playhead on the exact metronome grid. The seek also lands on a frame computed from the f64 beat
  time, not on a beat rounded to whole milliseconds by the frontend.

### 2.3 Jumps in the engine (requirements 3, 4, 6)

`PlayerManager::jump_to(&mut self, id: &str, position_ms: f64, beat: bool, now: Instant) -> Result<PlayerTransport, String>`
in `player.rs`, next to `skip`:

1. If `position_ms` is not finite, return `Err("bad position")`.
2. `dur = duration in ms (f64)`. `target = if beat { state.tempo.nearest_beat_ms(position_ms, dur).unwrap_or(position_ms) } else { position_ms }`,
   then `target.clamp(0.0, dur)`.
3. `f = s.set_pos(s.frame_of_ms_f(target))`. `set_pos` already clamps to `floor..=frames`, so the result
   is never before 0:00.0 (or the set first beat) and never past the end.
4. Send `Cmd::Jump(f)`, which is instant like Rew/FF: no 0.3 s scrub silence, and playback goes on while
   playing. Then `s.mark_dirty(now)`.
5. Log `calliope: player jump requested_ms=<x, 1 decimal> beat=<1|0> position_ms=<ms_of(f)>`.
6. Return `publish(&sink, s, now)`.

- **To start** = `jump_to(id, 0.0, false)`. One command serves both buttons and the click, so no new
  engine command is needed.
- **Rew / FF** already clamp to `floor..=end` (`skip` → `set_pos`). They are only covered by tests (§4).
- **Why a jump and not `seek`:** `seek` is the slider's scrub (0.3 s silence before resuming). Rew/FF and
  section jumps are instant, and a practice jump should sound at once (§2.6 b).
- **IPC:** a new `#[tauri::command] player_jump(app, id: String, position: f64, beat: bool) -> Result<PlayerTransport, String>`
  in `player_ipc.rs`, built like `player_skip` (`blocking` + `PlayerHost::with`). It is registered in
  `gui.rs` `generate_handler!`, in the `build.rs` `COMMANDS` list after `"player_skip"`, and as
  `"allow-player-jump"` in `capabilities/main.json` after `"allow-player-skip"`. The existing static
  test keeps the three lists in sync.

### 2.4 Frontend

- `ipc.ts`: `playerJump = (id: string, position: number, beat: boolean): Promise<PlayerTransport> => invoke<PlayerTransport>('player_jump', { id, position, beat })`,
  next to `playerSkip`.
- `player-state.svelte.ts`, next to `skip`:
  - `toStart()`: calls `transportCall('player_jump', (id) => playerJump(id, 0, false), (t) => \`player to-start position=${t.position_ms}\`)`.
  - `seekClick(ms: number)`: calls `transportCall('player_jump', (id) => playerJump(id, ms, true), (t) => \`player jump position=${t.position_ms} requested=${Math.round(ms)}\`)`.
- `PlaybackSection.svelte`: an outline "To start" button before Rew, in the same style as Rew
  (`h-[3.25rem] text-lg`, disabled while not active). The order is To start, Rew, Play/Pause, FF. The
  label is text, like Rew/FF, so the accessible name is "To start". An icon would look like the Sections
  group's "|<" (section start).
- `WaveformView.svelte`: a new optional prop `onseek?: (ms: number) => void`, documented as "Player
  mode: a click outside markers and labels; the ms under the pointer". In `onpointerdown`, free-move
  branch:
  - **No tempo:** today `active` is false at BPM 0 and the handler returns. Before the
    `if (!active) return;` add: `if (!hasTempo) { if (onseek) { e.preventDefault(); canvas?.focus(); onseek(msAt(x)); } return; }`.
    `editable` and `locked` are still checked first, at the top.
  - **With a tempo:** in the `b === null` branch call `onselect?.(sectionAt(x))` and then
    `onseek?.(msAt(x))`.
  - The label branch and the marker-drag branch do not call `onseek`. The Editor's `MixLane` passes
    neither `freeMove` nor `onseek`, so the Editor does not change.
- `PlayerPane.svelte`: pass `onseek={seekClick}`. Extend the `player layout` log line with
  `hscroll=${el.scrollWidth - el.clientWidth} playback_w=${Math.round(wd('Playback'))}`. The fields are
  appended, so the existing parsers (`field(&l, k)`) keep working.
- The Player slider guards (`dragging` / `keying` in `PlaybackSection.svelte`) are not touched.

### 2.5 Layout

At 1280x800 with the tree folded, the Playback pane is 965 px wide. Its first line (Rew, Play, FF, time,
slider `min-w-48 flex-1`) has room: the slider shrinks by the new button's width (about 105 px) and
nothing wraps, so the height and `overflow=0` stay. At 1024x640 the row is `flex-wrap`, so the slider may
wrap onto its own line. That is vertical only, and vertical scrolling at 1024x640 is allowed today. The
GUI test checks `overflow=0` at 1280x800 with the tree folded (the existing guarantee; with the tree
shown the pane already scrolls about 234 px). At 1024x640 it checks `hscroll=0` and
`playback_w <= width`. The new button shifts Play, FF and the time box on the Playback row, so the
window-coordinate constants for them are measured again (task 7).

### 2.6 Team decisions (not open questions; the owner may overrule)

- **a. To start goes to the playback floor.** That is 0:00.0, or the set first beat when one is set. Every
  position the manager sets is already clamped there (gui-first-beat requirement 3: the audio before a
  set first beat is never played). The spec's 0:00.0 is the track without a set first beat.
- **b. To start and waveform clicks are instant jumps** (like Rew/FF and the section jumps), not the
  slider's 0.3 s scrub. Playback goes on from the new position (requirement 6).
- **c. A click on the waveform while a BPM detection runs (`locked`) does nothing**, as today: the
  waveform is locked then.
- **d. Autodetect still carries the old first section's Count In** to the new first section. Only the
  Click is no longer carried: every new section is off.
- **e. The section BPM detection makes no sections.** It needs a tempo and changes the BPM of an existing
  section, keeping its flags. Nothing changes there; a test confirms that a detected BPM keeps the
  section's Click.
- **f.** A click before the first tempo segment snaps to the first tempo's grid extended backwards (the
  same beats the metronome uses), never below the floor.

## 3. Tasks

All commands run in `src/calliope-gui/`. Rust code is formatted with `cargo fmt --all` before each task
ends.

### Task 1: New sections start with Click off (Rust + every headless test that relied on the old default)

- **files:** `src/tempo_map.rs`, `src/tempo_detect.rs`, `src/player_state.rs` (tests only),
  `src/player_tests.rs`, `src/beat_detect_tests.rs`, `src/tempo_detect_tests.rs`,
  `tests/acceptance_player_backend.rs`, `tests/acceptance_beat_autodetect.rs`,
  `tests/acceptance_section_bpm.rs`, and any other headless test that `npm test` shows failing because
  of this change.
- **does:**
  - Add `SectionMeta::new_section()` as in §2.1 and use it in `set_bpm_at` (both branches; delete the
    "keeps the metronome flag" copy), `add_change` (delete the copy from `left`), and both segment
    constructions in `tempo_detect::detect`.
  - `with_play_settings_of`: set `s.meta.metronome = false` for every section and keep the Count In
    rule. Update its doc comment.
  - Update the `SectionMeta.metronome` doc comment: "missing key = on (older files); new sections start
    off (`new_section`)".
  - `tempo_map.rs` unit tests: change the tests that asserted the copy rule (`// Add copies the split
    section's metronome flag`, `// insert: copies the metronome flag of the split section`, the
    `with_play_settings_of` test). New assertions: the first BPM on an empty map gives
    `metronome == false`. A BPM-box insert and an Add into a section with Click on give a new section
    with Click off, and the split section keeps on. `with_play_settings_of(old)` with old Click on gives
    every section off, and the first section keeps old's Count In.
  - `player_state.rs` test (requirement 2, acceptance 4): load a schema-7 file with three sections:
    no key, `"metronome": false`, and `"metronome": true`. Check on, off, on. Save it and check the file
    has no key, `false`, and no key. Check that the saved segments never contain `"metronome": true`, so
    the previous build's parser reads them the same.
  - `player_tests.rs`:
    - (acceptance 1) Open PRACTICE (no tempo) and set 120 BPM through the manager's set-BPM action.
      The snapshot section has `metronome == false`. Play and pull 2 s: the output equals the decoded PCM
      after the fade (no clicks; pattern of `click_off_in_every_section_leaves_the_source_untouched`).
    - (acceptance 3) Add and a BPM-box split on a map whose section is on: the new section is off, and
      the file has `"metronome": false` on the new segment only.
    - (acceptance 4, manager level) Open a file with an on and an off section. Add a section, flush, and
      reopen: the old ones are unchanged and the new one is off.
  - (acceptance 2) `beat_detect_tests.rs` / `tempo_detect_tests.rs`: a detection over an empty map and
    over a map whose first section has Click on gives every section with Click off.
    `acceptance_section_bpm.rs` (or `player_tests.rs`): a detected BPM keeps the section's Click (on
    stays on).
  - Run `npm test`. Fix every failing test that relied on new sections clicking by switching Click on
    explicitly (`SectionEdit::Flags` / `set_section_flags`, or a fixture `player.json` without the key).
    Change copy-rule assertions to the new rule. Do not weaken unrelated assertions.
- **done when:** `cargo test --workspace` passes, including the new tests named above, and `npm test` is
  green.
- **test command:** `cargo test -p calliope-gui --lib tempo_map player_state player_tests beat_detect tempo_detect`,
  then `npm test`.
- **routine: no**

### Task 2: `TempoMap::nearest_beat_ms`

- **files:** `src/tempo_map.rs`
- **does:** implement §2.2. Unit tests:
  - 120 BPM at 0: `nearest_beat_ms(10200, 20000) == Some(10000)`, `10260 → 10500`, `10250 → 10000`
    (tie → earlier), `0 → 0`, `19990 → None or the last beat < 20000` (whichever the window yields;
    assert it is ≤ 20000).
  - Empty map → `None`. A non-finite `ms` → `None`.
  - Two sections (500 ms at 120, a detected change at 10500 at 90): a click at 11200 snaps to 11166.67
    (10500 + 666.67) within 1e-6.
  - Every result is an element of `beats_in(0, end)` (property check over a sweep of 0..end in 37 ms
    steps on the two-section map and on one with a stretched last beat).
  - A first segment at 300 ms at 120 BPM, click at 50 → `Some(300)`. The -200 ms beat is outside `0..`
    and is ignored.
- **done when:** `cargo test -p calliope-gui --lib tempo_map` passes. `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- **test command:** `cargo test -p calliope-gui --lib tempo_map`
- **routine: no** (the file is over 300 lines)

### Task 3: `jump_to` and the `player_jump` command

- **files:** `src/player.rs`, `src/player_ipc.rs`, `src/gui.rs`, `build.rs`, `capabilities/main.json`,
  `src/player_tests.rs`
- **does:** implement §2.3. `player_tests.rs`:
  - (acceptance 5) PRACTICE (20 s): play, seek to 15000, tick past 300 ms, `jump_to(0.0, false)`. The
    transport is playing, not `resume_pending`, at position 0. The next pulled block equals the decoded
    PCM from frame 0 after the fade (pattern of `seek_while_playing_is_silent_until_300ms_and_skip_clamps`).
  - (acceptance 6, extend the existing skip test) Paused at 2000, `skip(-5000)` → 0. At 17000 (3 s before
    the 20 s end), `skip(5000)` → 20000.
  - (acceptance 7) `with_tempo()` helper, or PRACTICE with 120 BPM set at 0 and the found start set so
    the first beat is 0. Paused: `jump_to(10200.0, true)` → `position_ms == 10000`, and the engine
    position frame equals `10000 * RATE / 1000` exactly. Playing: `jump_to(10200.0, true)` keeps
    playing, and the pulled audio equals the PCM from 10.0 s (no scrub silence).
  - (acceptance 8) No tempo: `jump_to(10200.4, true)` → `position_ms == 10200` (the frame of 10200.4 ms).
  - Range: `jump_to(-50.0, true)` → 0. `jump_to(99_999.0, false)` → 20000. `jump_to(f64::NAN, ..)` →
    `Err`. With a set first beat at 1200, `jump_to(0.0, false)` → 1200 (§2.6 a).
- **done when:** the new tests pass. The static command-list test (the one that keeps `gui.rs`,
  `build.rs` and `capabilities/main.json` identical) passes. `npm test` is green.
- **test command:** `cargo test -p calliope-gui --lib player_tests`, then `npm test`.
- **routine: no**

### Task 4: Frontend state: `playerJump`, `toStart`, `seekClick`

- **files:** `src/ui/lib/ipc.ts`, `src/ui/lib/player-state.svelte.ts`, `src/ui/lib/ipc-player.test.ts`,
  `src/ui/lib/player-state.test.ts`
- **does:** implement the §2.4 wrappers and the two functions, with their log lines. Tests:
  - `playerJump` invokes `player_jump` with `{id, position, beat}`.
  - `toStart()` sends `{position: 0, beat: false}`. `seekClick(10200)` sends
    `{position: 10200, beat: true}`.
  - Both update `pl.transport` from the answer and log `player to-start position=0` /
    `player jump position=10000 requested=10200`.
  - Both do nothing while `pl.status !== 'active'`.
- **done when:** `npx vitest run src/ui/lib/ipc-player.test.ts src/ui/lib/player-state.test.ts` passes and
  `npm run check` is clean.
- **test command:** `npx vitest run src/ui/lib/ipc-player.test.ts src/ui/lib/player-state.test.ts && npm run check`
- **routine: no** (`player-state.svelte.ts` is about 890 lines)

### Task 5: The To start button and the layout log fields

- **files:** `src/ui/components/player/PlaybackSection.svelte`, `src/ui/components/player/PlayerPane.svelte`
- **does:** three exact edits.
  1. In `PlaybackSection.svelte` replace the line
     `  import { nudge, pl, playLabel, seek, skip, togglePlay } from '$lib/player-state.svelte';`
     with
     `  import { nudge, pl, playLabel, seek, skip, toStart, togglePlay } from '$lib/player-state.svelte';`
  2. In `PlaybackSection.svelte` insert, directly above the line that starts with
     `  <Button variant="outline" class="h-[3.25rem] text-lg" disabled={!active} onclick={() => void skip(-5000)}>Rew</Button>`,
     this new line:
     `  <Button variant="outline" class="h-[3.25rem] text-lg" disabled={!active} onclick={() => void toStart()}>To start</Button>`
  3. In `PlayerPane.svelte`, in the `frontendLog(...)` template string, replace
     `` loops_w=${Math.round(wd('Loops'))}`); ``
     with
     `` loops_w=${Math.round(wd('Loops'))} hscroll=${el.scrollWidth - el.clientWidth} playback_w=${Math.round(wd('Playback'))}`); ``
  Change nothing else. Do not commit.
- **done when:** `npm run check` is clean and `npx vitest run src/ui/components/player/ControlBar.test.ts`
  passes (the existing Rew/FF tests find their buttons by name).
- **test command:** `npm run check && npx vitest run src/ui/components/player/ControlBar.test.ts`
- **routine: yes**

### Task 6: Click to seek in `WaveformView`, wired in the Player

- **files:** `src/ui/components/player/WaveformView.svelte`, `src/ui/components/player/PlayerPane.svelte`,
  `src/ui/components/player/WaveformView.test.ts`, `src/ui/components/player/ControlBar.test.ts`
- **does:** implement the §2.4 `onseek` prop and `onpointerdown` changes, and pass
  `onseek={seekClick}` in `PlayerPane.svelte`. Tests:
  - `WaveformView.test.ts`, extending `'a click on a label calls onlabel; a click elsewhere selects the section under the pointer'`:
    - With freeMove and segments, a click in section 1 away from bars calls `onselect(1)` and
      `onseek(ms)` once, with `ms = msAt(x)` (±1 ms for the mocked width).
    - A click on a label calls `onlabel` only, with no `onseek`.
    - A pointerdown within 8 px of a bar starts the drag (`onmove` after a move), with no `onseek`.
  - `'is read-only at BPM 0'` becomes: with no segments and `onseek` set, a click calls `onseek(msAt(x))`
    and nothing else (no select, no drag). Without `onseek` it is still read-only.
  - While `locked` a click calls neither `onseek` nor `onselect`.
  - In Editor mode (no freeMove) a click never calls `onseek`.
  - `ControlBar.test.ts`:
    - The Playback group's buttons are, in order, "To start", "Rew", the Play/Pause toggle and "FF".
    - Clicking To start invokes `player_jump` with `{position: 0, beat: false}`.
    - A click on the Player waveform (the `player-waveform` canvas, mocked width) invokes `player_jump`
      with `beat: true`.
    - The slider still sends no seek when the position changes without a drag or a key (the existing
      guard test stays green).
- **done when:** `npx vitest run src/ui/components/player/` passes and `npm run check` is clean.
- **test command:** `npx vitest run src/ui/components/player/ && npm run check`
- **routine: no**

### Task 7: GUI tests, coordinates and `docs/ui.md`

- **files:** `tests/gui_player_e2e.rs`, `tests/gui_player_acceptance_e2e.rs`, `tests/gui_editor_e2e.rs`
  (only `PLAYER_PLAY`), `docs/ui.md`
- **does:**
  - Measure again from a 1280x800 screenshot with the tree folded (`gui-shot`): the Playback row's `PLAY`,
    `TIME` and `REW` (and `PLAYER_PLAY` in `gui_editor_e2e.rs`). Add `TO_START` and `FF` constants. Rows
    above the Playback group do not move, unless the screenshot shows otherwise.
  - Rewrite the start of `tempo_metronome_count_in` (acceptance 1):
    - After typing 120 BPM, the file's segment has `"metronome": false`. Play for 2 s: the capture has
      no click onsets.
    - Press `SEC_CLICK` and wait for `player section flags index=0 metronome=true count_in=false`. Then
      the existing 500 ms check runs.
    - The count-in part then presses Click (now on → off) and Count In as before.
  - Add a GUI test `player_transport`:
    - **Setup.** `PL_TEMPO` with its `player.json` replaced in the temp copy by `{"schema_version":7,"tempo":{"beats_per_bar":4,"segments":[{"start_ms":0,"bpm":120,"found_ms":0,"metronome":false},{"start_ms":15000,"bpm":100,"detected_ms":15000,"metronome":false}]}}`.
      `found_ms` 0 stops the automatic Find start.
    - **1. Layout.** At 1280x800 with the tree folded, the last `player layout` line has `overflow=0` and
      `hscroll=0`. Take screenshot `player-transport-1280`.
    - **2. To start while playing (acceptance 5).** Type 0:12.0, press Play, wait 500 ms and press
      To start. The UI line `player to-start position=0` appears, and the transport stays playing
      (the backend `player jump ... position_ms=0` line, then no `player pause`). The capture WAV after
      the jump has sound within 50 ms (no scrub gap).
    - **3. Rew / FF (acceptance 6).** Pause. Type 0:02.0 and press Rew: `player skip delta=-5000`, and
      the time box reads 0:00.0 (the transport position is 0 in the next `player pause`/transport log).
      Type 0:17.0 and press FF: the position is 20000.
    - **4. Click to seek (acceptance 7).** Click the waveform at the x of 10.2 s. The x comes from the
      `wave_box` geometry: `left + 10200/20000·(right-left)`. Expect the backend line
      `player jump requested_ms≈10200 (±50) beat=1 position_ms=10000`. Then press `SEC_CLICK`: the flags
      line names `index=0` (the section under the click is the target). Click again at 17.3 s while
      playing: `position_ms=17400`, playback goes on (no `player pause`), and the next flags press names
      `index=1`.
    - **5. Marker and label (acceptance 9).** Drag the bar at 15 s by 20 px: a `player section move` line,
      and the position is unchanged (no `player jump` line). Click the label of section 0: the BPM
      overlay opens (screenshot), with no `player jump` line. Press Escape.
    - **6. No tempo (acceptance 8).** Pick `PL_PRACTICE` (no `player.json`) and click at 10.2 s:
      `player jump ... beat=1 position_ms=<p>` with `p == round(requested_ms)` and
      `|requested_ms - 10200| ≤ 50`.
    - **7. 1024x640 (acceptance 10).** Resize to 1024x640 in both tree states: `hscroll=0` and
      `playback_w <= width`. Take screenshots `player-transport-1024-rail` and
      `player-transport-1024-shown`. If the tree-shown case already had `hscroll > 0` before this
      feature (the Sections row is known to clip there), check it on `git stash` of tasks 5-6. If so,
      assert only `playback_w <= width` there, and record it in the log and report for the owner.
  - Fix the existing GUI tests broken by this feature:
    - Waveform clicks (`WAVE_SECTION0/1`) now also move the playhead. Re-type the time after them
      where a test depends on the position.
    - New sections are silent: press Click where a test expects clicks after typing a BPM, Add or
      Autodetect. Check `player_tempo_sections`, `player_section_metronome`, `player_phase3`,
      `visual_review` and the Editor's Autodetect test.
  - `docs/ui.md`:
    - Playback row: "To start, Rew, Play/Pause, FF, the time box and the position slider", plus the new
      measured x positions.
    - Section toggles: "A new section starts with Click off (the first BPM, Autodetect, the BPM box, Add);
      sections saved before keep their setting".
    - Waveform: "a click outside markers and labels selects the section and moves the playhead to the
      nearest beat (no tempo: the clicked position); while playing, playback goes on from there".
    - Layout: the `hscroll` / `playback_w` fields.
- **done when:** `npm run test:gui -- tempo_metronome_count_in player_transport player_tempo_sections player_section_metronome player_phase3 visual_review qa_p3_layout_never_clips_sideways player_first_beat player_first_beat_tuning player_section_bpm_detect player_speed loops`
  passes, `npm run test:gui` (all) passes, and `npm test` is green. The screenshots above are looked at:
  To start sits before Rew, nothing clips, and the Speed line is unchanged.
- **test command:** `npm run test:gui -- player_transport tempo_metronome_count_in`, then `npm run test:gui`
  and `npm test`.
- **routine: no** (GUI, visual checks)

## 4. Test strategy

- **Unit (Rust):**
  - `tempo_map.rs`: new-section defaults, `with_play_settings_of`, `nearest_beat_ms` against `beats_in`.
  - `tempo_detect` / `beat_detect`: detected sections off.
  - `player_state.rs`: old files keep on and off, and the save keeps the format.
  - `player_tests.rs` (manager + `ManualBackend`, exact PCM comparisons): To start / beat jump / no-tempo
    jump while playing and paused, the range clamps, the first-beat floor, no clicks after a first BPM.
- **Unit (frontend, vitest):**
  - `ipc-player.test.ts`, `player-state.test.ts`: payloads, logs, inactive guard.
  - `WaveformView.test.ts`: seek and select on one click, no seek on a marker, label, locked or in the
    Editor, seek at BPM 0.
  - `ControlBar.test.ts`: button order, To start payload, waveform click payload, slider guard
    unchanged.
- **End-to-end (GUI on a private X display, `npm run test:gui`):** `gui_player_e2e::player_transport` and
  the rewritten `tempo_metronome_count_in`. They use temp copies of `tests/fixtures/library-player` and
  the capturing fake audio backend (float32 WAV analysed for click onsets and gaps). The real sound
  device is never opened, and there are no dialogs. Waits stay short (`wait_contains`, a few 300-700 ms
  sleeps where the layout log settles).
- **No hardware or network:** the feature only touches the audio output, which the capture and
  `ManualBackend` fakes stand in for.
- **Whole headless suite:** `npm test` in `src/calliope-gui/` (fmt check, svelte-check, vitest, vite
  build, `cargo test --workspace`, clippy). **GUI suite:** `npm run test:gui`.

## 5. Deployment

Single machine: everything is in `calliope-gui` on the laptop (`npm run build:app`). No server change.

## 6. Manual checks (owner, on the laptop with the real audio device)

1. **New tempo is quiet.** Open a track with no tempo in the Player, type a BPM and press Play: no
   clicks. Press Click: the clicks start on the beat.
2. **Old tracks keep their clicks.** Open a track whose sections clicked before this build: they still
   click. A track that was silent stays silent.
3. **Add / split is quiet.** On a clicking section press `+` (or type a different BPM mid-section) and
   play across the new change: the new section is silent and the old one still clicks.
4. **Autodetect is quiet.** In the Editor run Autodetect BPM on a track, then play it in the Player: no
   clicks in any section.
5. **To start.** While playing in the middle of a song press To start: the song goes on from the start
   with no gap or glitch.
6. **Click to seek.** With Click on in a section, click the waveform between two beats while playing:
   the music goes on from the beat nearest the click and the click lands with the music. Click near a
   marker: it drags instead and the music doesn't jump.
7. **Ends.** Press Rew near the start and FF near the end: the position stops at 0:00.0 and at the end.
8. **Previous build.** Optional: open a track saved by this build with the previous Calliope build. The
   sections show the same Click settings.
9. **Layout.** At the usual window size the Player shows To start, Rew, Play, FF on one line without a
   scrollbar.

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests |
|---|---|---|
| No tempo, a BPM is set → section off, Play has no clicks | 1, 7 | `player_tests` first-BPM test (exact PCM); `tempo_map` empty-map test; GUI `tempo_metronome_count_in` |
| Autodetect / section BPM detection → every new section off | 1 | `beat_detect_tests` / `tempo_detect_tests` detection tests, `with_play_settings_of` test; the detection-keeps-Click test (§2.6 e) |
| Split or Add → new section off | 1 | `tempo_map` insert/Add tests; `player_tests` Add + split test (file has `false` on the new segment) |
| Previous build's `player.json` (no key / false) keeps settings, also after a save | 1 | `player_state` three-section load/save test; `player_tests` reopen test |
| 1:23.4 playing, To start → goes on from 0:00.0 | 3, 4, 5, 6, 7 | `player_tests` To start while playing (15 s in the 20 s fixture); `ControlBar.test.ts`; GUI `player_transport` step 2 |
| 0:02.0 Rew → 0:00.0; 3 s before the end FF → end | 3, 7 | `player_tests` skip test; GUI `player_transport` step 3 |
| 120 BPM, click at 0:10.2 → 0:10.0, section selected, keeps playing | 2, 3, 6, 7 | `tempo_map` `nearest_beat_ms`; `player_tests` beat jump; `WaveformView.test.ts`; GUI step 4 |
| No tempo, click at 0:10.2 → 0:10.2 | 3, 6, 7 | `player_tests` no-tempo jump; `WaveformView.test.ts` BPM 0; GUI step 6 |
| Marker drags / label renames, playhead does not move | 6, 7 | `WaveformView.test.ts`; GUI step 5 |
| 1280x800 no scrollbar; 1024x640 no horizontal scroll | 5, 7 | GUI steps 1 and 7 (`overflow=0`, `hscroll=0`, `playback_w <= width`), `qa_p3_layout_never_clips_sideways` |

## 8. Open questions

None. See §2.6 for the decisions taken by the team.
