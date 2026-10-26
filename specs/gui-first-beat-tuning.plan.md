# Plan: gui-first-beat-tuning (keyboard readout, start offset field, Find start in section 1, Prev / Next section)

Spec: `specs/gui-first-beat-tuning.md` (owner answers 2026-10-26 in its Open questions section).
Base: `021f128` on `spec/gui-first-beat-tuning`. Paths follow "Paths in this document" of `docs/architecture.md`:
`src/*.rs`, `src/ui/...` and `tests/...` are inside `src/calliope-gui/`.

## 1. Summary

This feature only touches the frontend. In the Player and the Editor, a key move of a section start bar now shows the drag
readout. It stays up for 1.5 s after the last key, or until the waveform loses focus. The Player's Sections pane gets a
fourth row: a "Start" field with the target section's offset in 1/64 beats, the time next to it, and the "Find start"
button, which moves there from the button row and works only in the first section. The button row gets "Prev" and "Next"
section buttons. Both features reuse the existing `player_tempo` actions: `move-change` with absolute steps, clamped in
the frontend like a drag, and `jump-section` with index ± 1 and edge `start`. No Rust code, IPC type or file format
changes.

## 2. Design

### 2.1 What already exists and is reused

- **Offsets.** `waveform-geometry.ts`: `moveRefMs(segs, i)` gives the reference: `found_ms ?? start_ms` for index 0,
  else `detected_ms ?? segs[i-1].start_ms`. `stepBpm(segs, i)` gives the BPM of one step (the section's own BPM for 0,
  else the left section's). `offsetSteps(start, ref, bpm)` rounds the offset. `dragSteps(...)` and `firstBeatSteps(...)`
  are the drag clamps, mirroring `TempoMap::move_change` / `move_first_beat`. `moveLimit` gives the ±256 (widened) limit
  of detected changes. These are the same functions the readout and the drag use. The field therefore counts "from the
  same point as the drag readout" by construction (requirement 2).
- **Moves.** Player `SectionEdit::Move { index, at, steps }` (`player_tempo` `{kind: 'move-change', index, at, steps}`)
  takes **absolute** steps from that reference. It is guarded by `at` (the section's current start; a stale `at` is
  refused with "The tempo sections changed; try again"). Index 0 runs `move_first_beat` and clamps to 0 .. next − 1 beat.
  Other indexes run `move_change`, which clamps to one beat from the neighbours and the track end, but **refuses** a
  detected change's |steps| above its limit. The frontend therefore clamps before it sends (§2.3), exactly as the drag
  does.
- **Jumps.** `player_tempo` `{kind: 'jump-section', index, at, edge}` → `PlayerManager::jump_section`. It works for
  any index. Start of section 0 is `play_start_ms()`: the first beat when it is set, else 0:00. While playing, it is a
  `Cmd::Jump`: playback continues without a gap, and a running pre-roll count-in is cancelled
  (`jump_section_cancels_a_pre_roll_count_in`). While paused, it only moves the playhead, and the next Play counts in
  per the section's Count In. Prev / Next send this same action for the neighbouring section, so they behave like |◀ in
  every case, count-in included (requirement 4.3). No backend change is needed (see open question 1 about the
  acceptance wording).
- **Target section.** `targetSection()` in `player-state.svelte.ts` is the selection made on the waveform, until the
  playhead enters another section, else the section under the playhead. The whole pane already works on it. The field,
  Prev / Next and the Find start rule use it too, so moving the playhead into another section shows that section's
  offset (requirement 2, last sentence).

### 2.2 Keyboard readout (`WaveformView.svelte`, requirement 1)

Today `readout` is derived from `drag` only. A new piece of state `keyShow: { index: number; steps: number } | null`
works as follows:

- **Shown by the keys.** Every Left/Right key press on a movable bar (both modes; Shift as well) sets
  `keyShow = { index: sel, steps }`. `steps` is the clamped target the handler computes (`stepsFor(...)`). This also
  applies when the press does not move the bar because it is already at a limit: the handler sets `keyShow` with the
  current `base` before its early `return`. The readout then stays up and shows the limit value.
- **The timer.** Each press clears the timer and starts `setTimeout(() => (keyShow = null), KEY_READOUT_MS)`, with
  `KEY_READOUT_MS = 1500`, exported from `waveform-geometry.ts` so tests use the same value.
- **When it goes away.** `keyShow` and its timer are cleared on blur, on pointerdown (a drag replaces it), when
  `keyShow.index >= segments.length` (the same `$effect` that drops a stale drag), and on destroy (the `onMount` cleanup).
- **Same text, same place.** The readout text is computed from `drag ?? keyShow` with the existing formula: index 0
  gives "+N/64 beat (+m:ss.mmm) from the found start", the others "+N/64 beat (time)" plus " · limit ±4 beats" when the
  limit is finite. The existing `<div data-testid="{testid}-readout">` (top left of the waveform) renders when
  `drag || keyShow`.

Why the asked steps and not the snapshot's: it is the same thing the drag preview shows, it updates on every press
without waiting for the backend, and `stepsFor` already applies the backend's clamp.

### 2.3 Start offset field (requirement 2)

**Pure helpers.**

`src/ui/lib/section-meta.ts`, next to the other section helpers:

- `parseOffset(text): number | null`:
  1. Trim the text and replace U+2212 with `-`.
  2. Match `/^[+-]?\d+$/`; any other text gives `null`.
  3. Return `Math.max(-1e9, Math.min(1e9, Number(t)))`, with `-0` normalised to `0`.

  So `1.5`, `abc`, `+`, `1e3` and an empty text are refused. A huge whole number is accepted and clamped later
  ("clamped like a drag").
- `formatOffset(steps): string`: `+10`, `0`, `-8` (the readout's `signed`).
- `offsetTime(ms): string`: `+0:00.078`, `-0:00.063`, `+0:00.000`. The sign is `-` only when `ms < 0`, followed by
  `m:ss.mmm` of `|ms|`, formatted like WaveformView's `fmt(.., 3)`.

`src/ui/lib/waveform-geometry.ts`:

- `sectionOffset(segs, i): { steps: number; ms: number }`. `steps = offsetSteps(segs[i].start_ms, moveRefMs(segs, i),
  stepBpm(segs, i))`, and `ms = segs[i].start_ms − moveRefMs(segs, i)`. The ms is the actual applied offset, so after a
  clamp to 0:00 the time reads exactly `-0:01.200`.
- `clampMoveSteps(segs, i, steps, durationMs): number`, the drag clamp for a wanted step count, with `ref =
  moveRefMs(segs, i)`:
  - **`i === 0`.** With `[lo, hi] = firstBeatLimits(segs, durationMs)`:
    `firstBeatSteps(ref, ref + steps·fineMs(segs[0].bpm), segs[0].bpm, lo, hi)`.
  - **Other `i`.** With `prev = segs[i-1]` and `fine = fineMs(prev.bpm)`:
    `dragSteps(ref, ref + steps·fine, prev.bpm, prev.start_ms + 60000/prev.bpm, (segs[i+1]?.start_ms ?? durationMs) −
    60000/segs[i].bpm, moveLimit(segs, i))`.
  - **Why `durationMs` and not ∞ for the last section.** It is the same bound the backend uses (`end_ms`), so the field's
    clamp and the backend's agree.
- `KEY_READOUT_MS = 1500`.

**State (`player-state.svelte.ts`).**

- `pl.offsetEdit: { index: number; at: number } | null` is captured on focus (`beginOffsetEdit()`), like `pl.nameEdit`:
  while the field has the focus, the pane shows the captured section, and the playhead crossing a boundary during
  playback cannot redirect the edit. It is cleared by `cancelOffsetEdit()` and by `resetFields()`.
- `commitSectionOffset(text: string): Promise<boolean>` takes the captured edit, else `targetSection()`, and works in
  these steps:
  1. **Invalid text.** If `parseOffset` gives `null`: set
     `pl.sectionError = 'Type a whole number of 1/64 beats, like +10 or -8'`, log
     `player section offset refused text=<t>`, return `false`. Nothing is sent.
  2. **Clamp.** `steps = clampMoveSteps(segs, index, n, pl.snap.duration_ms)`.
  3. **No change.** If `steps === sectionOffset(segs, index).steps`: log
     `player section offset index=<i> text=<t> steps=<s> unchanged`, clear `sectionError`, return `true`.
  4. **Move.** Otherwise log `player section offset index=<i> text=<t> steps=<s>`, then
     `sectionCall('move', index, (id) => playerTempo(id, {kind: 'move-change', index, at, steps}))` with the captured
     `at`. It is not coalesced with `moveChange`: one typed value is one call. `sectionCall` already logs
     `player section move index=<i>` plus `player sections ...`, puts a refusal into `sectionError`, and applies the
     snapshot, so the field shows the applied value.
  5. Return `true`.

**Component `src/ui/components/player/SectionOffset.svelte` (new, about 70 lines).**

This is the pane's fourth row: `<div class="flex flex-wrap items-center gap-2" role="group" aria-label="Section start">`.

- **Label.** `<label for="section-offset">Start</label>` (`text-base`).
- **The field.** `<input id="section-offset" type="text">`:
  - Classes: `h-9 w-16 rounded-md border border-input bg-background px-1 text-center text-lg tabular-nums`.
  - `aria-label="Start offset of the section in 1/64 beats"` and `aria-invalid` while refused.
  - `title`: "Type a BPM first" without a tempo, else "1/64 beats from the found start" (section 1), "from the detected
    position" (detected) or "from the previous section's start" (typed).
  - Disabled without a tempo.
  - `data-testid="section-offset"`.
- **Unit and time.** `<span>/64 beat</span>` (`text-base text-muted-foreground`), then the time
  `<span data-testid="section-offset-time" class="text-base tabular-nums">` with `offsetTime(ms)`.
- **Find start.** The existing button, moved here from the button row. `h-9 px-1.5 text-sm`, the same aria-label and
  action. It is disabled when there is no tempo, when the target section is not section 1, or while it runs. Its titles
  are "Type a BPM first", "Find start works in the first section", and "Finding the start...".
- **Field behaviour** (local `text: string | null`; the field shows `text ?? formatOffset(sectionOffset(segs,
  target).steps)`):
  - **Focus.** `beginOffsetEdit()`, then `text = null`.
  - **Input.** Store the text.
  - **Enter.** `commitSectionOffset(text)`. If it gives `true`: `text = null` and blur. If `false` (invalid): keep the
    text, set aria-invalid, and keep the focus.
  - **Escape.** `text = null`, `cancelOffsetEdit()`, clear `sectionError` if it came from the field, then blur.
  - **Blur.** If the text is unchanged, nothing is sent. Otherwise, if the text is still dirty, commit once; when that
    gives `false`, the message stays and `text = null` (the field shows the current value again). Then
    `cancelOffsetEdit()`.
  - **Keys.** The Player's global keys already ignore `input` elements (`PlayerView.svelte` `OWN_KEYS`).

**`SectionsSection.svelte`:**

- **Target.** `target` includes `pl.offsetEdit` the same way it uses `pl.nameEdit`: `pl.nameEdit ?? pl.offsetEdit`.
- **Rows.** Row 4 is `<SectionOffset {target} />`, placed between row 3 (Click / Count In / status) and the message line.
- **Find start.** The button moves into row 4 (see above).

### 2.4 Prev / Next section (requirement 4)

- **`stepSection(dir: -1 | 1)`** in `player-state.svelte.ts` works in these steps:
  1. `from = targetSection()` and `to = from + dir`. When it is out of range or there is no `at`, return.
  2. Call
     `sectionCall('jump', to, (id) => playerTempo(id, {kind: 'jump-section', index: to, at: startOf(to), edge: 'start'}), () => { pl.section = to; }, (s) => `edge=start position=${s.transport.position_ms} via=${dir < 0 ? 'prev' : 'next'}`)`.
  3. Setting `pl.section = to` makes the pane show the target section at once, also during a jump that lands exactly
     on the boundary. `dropSelectionOnMove` keeps it, because the playhead is now in `to`.

  "Prev goes to the previous section's start wherever the playhead is inside its section": there is no
  "restart this section first" rule.
- **Buttons.** In row 2, the order becomes `+ − ×2 ÷2 Prev |◀ ▶| Next`. Prev and Next are text buttons,
  `h-[3.25rem] w-11 px-0 text-base`:
  - aria-labels "Go to the start of the previous section" and "Go to the start of the next section".
  - Prev is disabled when there is no tempo or the target is 0 (title "This is the first section").
  - Next is disabled when there is no tempo or the target is the last section (title "This is the last section").
  - "Prev" / "Next" are the words the Loops pane already uses for its loop navigation.
- **Start of the first section.** For Prev into section 1 it is `section_jump_ms(0, Start)`. That is the first beat when
  it is set, as for |◀ and Home. For a legacy track whose first beat is not set it is still 0:00, consistent with
  "plays as before" (requirement 6 of gui-first-beat).

### 2.5 Layout at 1280x800 (requirement 5)

Measured today (`target/gui-shots/player-dark-layout.txt`, root font 18 px, tree folded): the pane is 965 px wide, with
`sections_w=496` and `loops_w=424`. Sections is `flex-none`, so it is as wide as its widest row. Its inner width is
about 465 px, and both the title row and the button row are about that wide. Loops has `basis-[23rem]` (414 px), so
Sections can grow by about 10 px at most before Loops wraps below. The Sections pane is stretched to the height of
Loops (250 px). Its own content needs about 225 px, which leaves about 25 px of vertical slack.

| Row | Today | After | Width (estimate) |
|---|---|---|---|
| 1 title | Sections, BPM, V, name box | unchanged | about 465 px |
| 2 buttons | 6 × `w-12` (54 px) + Find start (`px-1.5 text-sm`, about 84 px) = about 462 px | 8 × `w-11` (49.5 px) + 7 gaps × 9 px = **459 px** (Find start leaves) | ≤ 465 px, so Sections does not widen |
| 3 | Click, Count In, status | unchanged | about 410 px |
| 4 (new) | none | Start, field (72 px), "/64 beat", time, Find start (`h-9`) | about 395 px |
| message | `min-h-5` line | unchanged | full |

- **Width.** All eight row-2 buttons become `w-11` (2.75 rem, 49.5 px), so Prev and Next fit without widening the
  pane. That is the only way to keep 8 buttons within the current 465 px. The alternatives were to keep Find start in
  row 2 and put Prev / Next in row 4, away from |◀ ▶|, or to wrap row 2.
- **Height.** Row 4 adds 40.5 + 4.5 px. Minus the 25 px of slack, the controls grow by about **20 px**. The tablature
  placeholder (`flex-1`, at least `min-h-9`, now about 130 px tall) absorbs that, so `overflow` stays 0. As a result,
  the waveform, the Sections / Loops row and the Playback row move **up by the same amount**. Every window coordinate
  below the placeholder in the GUI tests therefore has to be measured again (Task 6). No row wraps at 1280x800, which
  is what requirement 5 allows.
- **The check.** The `player layout` line must read `overflow=0`, `loops_beside=1` and `sections_w ≤ 496` at 1280x800
  with the tree folded. It is asserted in the new GUI test and in `visual_review`. If the implementer measures a wider
  row 2 (fonts), the fallback is `text-sm` on Prev / Next. The 8 × `w-11` stays.

### 2.6 Logs (frontend, via `frontend_log`)

New: `player section offset index=<i> text=<t> steps=<s>[ unchanged]` and `player section offset refused text=<t>`.
Changed: Prev / Next give `player section jump index=<to> edge=start position=<ms> via=<prev|next>`. The |◀ / ▶| line
stays as it is. The backend lines are unchanged: `calliope: player section move ... index= ... steps= applied=` and
`calliope: player section jump id= index= edge= position_ms=`.

## 3. Tasks

Every task follows these rules:

- Run only the tests the task touches (named below). The lead runs `npm test` once per task before committing.
- `npm` commands run in `src/calliope-gui/`.
- No Rust source changes in this feature except GUI test files. Those are formatted with `cargo fmt --all`.

### Task 1. Offset text helpers
- routine: yes
- files: `src/ui/lib/section-meta.ts`, `src/ui/lib/section-meta.test.ts`.
- does: add, with JSDoc, exactly as §2.3.
  - **`parseOffset(text: string): number | null`:**
    1. Set `t = text.trim().replace(/−/g, '-')`.
    2. If `!/^[+-]?\d+$/.test(t)`, return `null`.
    3. Set `n = Math.max(-1e9, Math.min(1e9, Number(t)))` and return `n === 0 ? 0 : n` (no `-0`).
  - **`formatOffset(steps: number): string`:** `steps > 0 ? `+${steps}` : `${steps}``.
  - **`offsetTime(ms: number): string`:**
    1. Set `sign = ms < 0 ? '-' : '+'` and `total = Math.abs(ms) / 1000`.
    2. Set `m = Math.floor(total / 60)` and `s = (total - m * 60).toFixed(3)`.
    3. Return `${sign}${m}:${Number(s) < 10 ? '0' : ''}${s}`.
- tests:
  - `parseOffset`: `'+10'`→10, `'10'`→10, `' -8 '`→-8, `'−8'`→-8, `'0'`→0, `'-0'`→0 (check `Object.is(.., 0)`),
    `'1.5'`→null, `'abc'`→null, `''`→null, `'+'`→null, `'1e3'`→null, `'99999999999999999999'`→1e9,
    `'-99999999999999999999'`→-1e9.
  - `formatOffset`: 10→`'+10'`, 0→`'0'`, -8→`'-8'`.
  - `offsetTime`: 78.125→`'+0:00.078'`, -62.5→`'-0:00.063'`, 0→`'+0:00.000'`, -1200→`'-0:01.200'`, 65000→`'+1:05.000'`.
- done when: `npx vitest run src/ui/lib/section-meta.test.ts` passes; `npm run check` is clean.

### Task 2. Geometry helpers for the field and the readout timer
- routine: yes
- files: `src/ui/lib/waveform-geometry.ts`, `src/ui/lib/waveform-geometry.test.ts`.
- does: add, with JSDoc (types: the file's `MoveSeg`):
  - `export const KEY_READOUT_MS = 1500;` (how long the keyboard readout stays after the last key).
  - `sectionOffset(segs, i)`: `const ref = moveRefMs(segs, i); return { steps: offsetSteps(segs[i].start_ms, ref,
    stepBpm(segs, i)), ms: segs[i].start_ms - ref };`.
  - `clampMoveSteps(segs, i, steps, durationMs)`: `const ref = moveRefMs(segs, i);`.
    - When `i === 0`:
      `const [lo, hi] = firstBeatLimits(segs, durationMs); return firstBeatSteps(ref, ref + steps * fineMs(segs[0].bpm),
      segs[0].bpm, lo, hi);`.
    - Otherwise:
      `const prev = segs[i - 1]; const next = segs[i + 1]; const lo = prev.start_ms + 60000 / prev.bpm; const hi =
      (next ? next.start_ms : durationMs) - 60000 / segs[i].bpm; return dragSteps(ref, ref + steps * fineMs(prev.bpm),
      prev.bpm, lo, hi, moveLimit(segs, i));`.
- tests: use one fixture `S` for all of them:
  `[{start_ms: 1200, bpm: 120, found_ms: 1200}, {start_ms: 3000, bpm: 120, detected_ms: 3000}, {start_ms: 4200, bpm: 120}]`,
  duration 5200 (1/64 beat = 7.8125 ms).
  - `clampMoveSteps`:
    - `(S, 0, 10)` → 10; `(S, 0, -9999)` → -154; `(S, 0, 9999)` → 167.
    - `(S, 1, -8)` → -8; `(S, 1, -9999)` → -166; `(S, 1, 9999)` → 89.
    - `(S, 2, 9999)` → 217; `(S, 2, -9999)` → 64.
    - `(S, 0, 1e9)` → 167.
  - `sectionOffset`:
    - `(S, 0)` → `{steps: 0, ms: 0}`; `(S, 2)` → `{steps: 154, ms: 1200}`.
    - With `S[0].start_ms = 1278.125`: `(.., 0)` → `{steps: 10, ms: 78.125}`.
    - With `S[0].start_ms = 0`: `{steps: -154, ms: -1200}`.
    - With `S[1].start_ms = 2937.5`: `(.., 1)` → `{steps: -8, ms: -62.5}`.
  - `KEY_READOUT_MS === 1500`.
- done when: `npx vitest run src/ui/lib/waveform-geometry.test.ts` passes; `npm run check` is clean.

### Task 3. Keyboard readout in the waveform
- routine: no
- files: `src/ui/components/player/WaveformView.svelte`, `src/ui/components/player/WaveformView.test.ts`.
- does: §2.2 (`keyShow`, the timer with `KEY_READOUT_MS`, clearing on blur / pointerdown / stale index / destroy, and
  a readout at the limit). The readout and the `{#if}` use `drag ?? keyShow`. Drag behaviour is unchanged.
- tests: with `vi.useFakeTimers()`, canvas spied as today.
  - **Player mode, first bar.** `found_ms` 1200 at 120 BPM. Focus, Right gives the readout
    `+1/64 beat (+0:00.008) from the found start`.
    - It is still there after 1499 ms.
    - A second Right at 1000 ms shows `+2/64 beat (+0:00.016) from the found start`, still there 1400 ms later, and gone
      1500 ms after that press.
    - Blur hides it at once.
  - **Editor mode (no `freeMove`), a detected change selected with Down.**
    1. Drag it one fine step and read the readout text during the drag. Then pointerup.
    2. Press Left / Right until the key target reaches the same offset (or move from 0 by one step and compare it with
       a one-step drag from the same start).
    3. The text equals the drag's text, including the ` · limit ±4 beats` tail.
  - **At the limit.** The first bar at its 0:00 limit, then Left: the readout shows (with the limit offset) and no
    `onmove` is called.
  - **Pointerdown** while the key readout shows: it is replaced by the drag readout, and gone after pointerup.
  - The existing drag and readout tests keep passing.
- done when: `npx vitest run src/ui/components/player/WaveformView.test.ts src/ui/components/editor` passes;
  `npm run check` is clean.

### Task 4. Player state: offset commit and Prev / Next
- routine: no
- files: `src/ui/lib/player-state.svelte.ts`, `src/ui/lib/player-state.test.ts`.
- does: §2.3 state (`pl.offsetEdit`, `beginOffsetEdit`, `cancelOffsetEdit`, `commitSectionOffset`, reset in
  `resetFields`) and §2.4 `stepSection`, with the logs of §2.6.
- tests (mockIPC as the file does today), with the three-section snapshot `S` from Task 2:
  - **Valid text.** Target 0, `commitSectionOffset('+10')` sends `{kind: 'move-change', index: 0, at: 1200, steps: 10}`
    and resolves `true`.
  - **Too large.** `'-9999'` sends steps -154. For target 1, `'-9999'` sends steps -166, never more than the limit.
  - **Invalid text.** `'1.5'` sends nothing, resolves `false`, and `pl.sectionError` is the message.
  - **No change.** The current value sends nothing and resolves `true`.
  - **Captured edit.** `beginOffsetEdit()` on target 1. Then a transport event moves `tempo_index` to 2. Then
    `commitSectionOffset('-8')` sends `index: 1, at: 3000, steps: -8`.
  - **Next.** Target 1 (transport index 1) sends `{kind: 'jump-section', index: 2, at: 4200, edge: 'start'}`. The
    result: `pl.section === 2` and a log line containing `via=next`.
  - **Prev.** Target 1 sends `index: 0, at: 1200`. Prev on target 0 and Next on target 2 send nothing.
  - **Same payload as |◀.** For target 1, Next sends the same payload as `jumpSection('start')` would send with target 2.
  - **Track switch.** A reply arriving after a track switch is ignored (the existing generation guard via `sectionCall`).
- done when: `npx vitest run src/ui/lib/player-state.test.ts` passes; `npm run check` is clean.

### Task 5. Sections pane: Prev / Next, the Start row, Find start in section 1
- routine: no
- files:
  - `src/ui/components/player/SectionOffset.svelte` (new).
  - `src/ui/components/player/SectionsSection.svelte`.
  - `src/ui/components/player/ControlBar.test.ts` (its "Sections group" describe).
- does: §2.3 component, §2.4 buttons, §2.5 classes. All eight row-2 buttons get `w-11`. Find start moves into
  `SectionOffset`. Requirement 3: Find start is disabled outside section 1, with the title
  "Find start works in the first section".
- tests (mockIPC, three-section snapshot `S`):
  - **The row.** It shows `0` and `+0:00.000` for section 1.
  - **`+10` and Enter.**
    1. Typing `+10` and Enter sends the move.
    2. The answering snapshot (start 1278.125) makes the field show `+10` and the time `+0:00.078`.
    3. The field no longer has the focus.
  - **Text `x` and Enter.** The message line (`section-message`) shows the refusal with role alert, the field keeps `x`
    and has `aria-invalid`, and no call is made. A blur then puts the shown value back.
  - **A transport event moving `tempo_index` to 1** makes the field show section 1's offset (`0`, detected at 3000).
  - **The field follows the other moves.** A snapshot from a key move (`moveChange(0, 3)`, answered with start
    1223.4375) makes the field show `+3`. A `find-start` reply with start 1200 makes it show `0` again.
  - **Section 2 as target.**
    - Find start is disabled and has the title "Find start works in the first section".
    - The field is enabled.
    - `-8` sends `index: 1, steps: -8`.
  - **Prev / Next disabled states.** Prev is disabled in section 1 and Next in section 3. Both are enabled in section 2,
    and clicking them sends the jumps.
  - **Find start** still sends `{kind: 'find-start'}` from section 1 (its existing test is moved to the new place).
  - **No tempo.** The field, Prev, Next and Find start are disabled with "Type a BPM first".
- done when: `npx vitest run src/ui/components/player src/ui/lib/player-state.test.ts` passes; `npm run check` is
  clean; `npm run test:ui` passes.

### Task 6. Player GUI tests: new positions and the tuning flow
- routine: no
- files: `tests/gui_player_e2e.rs`, `tests/gui_player_acceptance_e2e.rs`.
- does:
  1. **Measure the new window coordinates** at 1280x800 with the tree folded. Use screenshots of the dark theme, or
     `DISPLAY=:1` and `xdotool getmouselocation` on a running app. Update:
     - every constant at or below the waveform (`PLAY`, `TIME`, `REW`, `SPEED_*`, `TRAIN_OPEN`, `LOOP_*`, `MARK_*`,
       `SEC_*`, `WAVE_*`), and the `tall` term of `wave_box`;
     - `train_pos` / `TRAIN_OPEN_1024` if the sweep shows they moved;
     - `FIND_START` (row 4);
     - in `gui_player_acceptance_e2e.rs`: `PLAY`, `TIME`, `SEC_CLICK`, `SEC_COUNT_IN`, `LOOP_ADD`, `MARK_END`,
       `LOOP_PLAY`.

     Add new constants: `SEC_PREV`, `SEC_NEXT`, `SEC_OFFSET`. Everything above the placeholder stays (`BLANK`,
     `TRIGGER`, `ITEM_*`, `BPM`, `SEC_NAME`). Update the comment above the Sections constants (eight buttons, a fourth
     row).
  2. **New test `player_first_beat_tuning`** (`#[test] #[ignore]`, like its neighbours). It uses a helper
     `tuning_dirs(test)`: `noisy_dirs`, with `player.json` rewritten as schema 7:
     `{"start_ms":1200,"bpm":120,"found_ms":1200}`, `{"start_ms":3000,"bpm":120,"detected_ms":3000,"count_in":true,"metronome":false}`,
     `{"start_ms":4200,"bpm":120,"metronome":false}`. Section 1 also has `"metronome":false`.
     1. **Open and check the layout.** The last `player layout` line has `overflow=0`, `loops_beside=1` and
        `sections_w` ≤ 496. Take screenshot `player-tuning.png`.
     2. **Keyboard readout.** Click the waveform at the x of 2000 ms (selects and focuses section 1) and press Right.
        Expect the line `calliope: player section move` with `index=0 steps=1 applied=1`. Within 400 ms take screenshot
        `player-tuning-key-readout.png`. Wait 2000 ms and take `player-tuning-key-readout-gone.png`.
     3. **Typed offset.** Click `SEC_OFFSET`, press `ctrl+a`, type `+10` and press Return. Expect the frontend line
        `player section offset index=0 text=+10 steps=10` and the backend move with `steps=10 applied=10`.
        `wait_json`: segment 0 has `start_ms` 1278.125 (± 0.01) and `found_ms` 1200.
     4. **Beyond the limit.** Type `-9999` and Return: `wait_json` gives `start_ms` 0.0. Take screenshot
        `player-tuning-clamped.png` (the field reads `-154`, the time `-0:01.200`). Then type `+10` again (back to
        1278.125).
     5. **Invalid text.** Type `1.5` and Return. Expect `player section offset refused text=1.5`. `player.json` is
        unchanged after 500 ms. Take screenshot `player-tuning-invalid.png` (the message line). Then press Escape.
     6. **Next.** Click `BLANK`, then `SEC_NEXT`. Expect `player section jump index=1 edge=start position=3000 via=next`
        (± 1 ms). Take screenshot `player-tuning-section2.png`: Find start is dimmed (disabled), the field reads `0`.
     7. **Offset in section 2.** In the field, type `-8` and Return. Expect the backend move with `index=1 steps=-8
        applied=-8`. `wait_json`: segment 1 `start_ms` 2937.5, segment 0 still 1278.125, segment 2 still 4200.
     8. **Prev from mid-section.** Enter `0:03.5` in `TIME` (inside section 2), then `SEC_PREV`. Expect the jump line
        `index=0 ... via=prev` with a position within 1 ms of 1278.
     9. **Last section.** Press `SEC_NEXT` twice. Expect the jump lines for indexes 1 and 2. Take screenshot
        `player-tuning-last.png` (Next disabled).
     10. **While playing.** Press Prev twice to get back to section 1, and capture. Press Play. The section has Count In
         off, so it starts at once. After 300 ms press `SEC_NEXT`, wait 1500 ms, and press Play to pause. Then repeat
         with |◀ (`SEC_START`) pressed while playing in section 2.
         - In both cases no `calliope: player transport playing=false` line appears between the jump and the pause
           (playback goes on).
         - The captured audio has the 440 Hz tone continuing within 60 ms of the jump, and no click onsets (Click is
           off everywhere).
         - That is, Next behaves like |◀ (open question 1 holds the alternative).
     11. **Layout again.** Repeat the layout assertion.
  3. **Existing tests.** `player_first_beat` uses the moved `FIND_START`. Its step 7 layout assertion stays.
- done when:
  - `npm run test:gui -- player_first_beat_tuning player_first_beat` passes (or `DISPLAY=:1 npm run test:gui:serial --`
    with the same names).
  - The whole `gui_player_e2e` and `gui_player_acceptance_e2e` binaries pass (the constants moved): `npm run test:gui --
    --test gui_player_e2e --test gui_player_acceptance_e2e`, or the serial form.
  - `CALLIOPE_GUI_SHOTS=1` `visual_review` was run once, and its 1280x800 rail assertion passes.
  - The screenshots were looked at:
    - the readout is visible in `player-tuning-key-readout.png` at the top left, with "+1/64 beat (+0:00.008) from the
      found start", and gone in `-gone`;
    - row 4 is aligned and fully visible, and nothing is clipped in row 2 ("Prev" / "Next" readable);
    - Find start is visibly disabled in section 2;
    - the message line shows the refusal.
  - `cargo fmt --all --check` is clean.

### Task 7. Editor GUI test: keyboard readout
- routine: no
- files: `tests/gui_editor_e2e.rs` (extend `editor_first_beat`).
- does:
  1. After the first Right and its `editor tempo move` line, take screenshot `editor-first-beat-key-readout.png` within
     400 ms (the readout "+1/64 beat (+0:00.008) from the found start" is shown top left of the Mix waveform).
  2. Wait 2000 ms before the existing `shot("editor-first-beat")`, which now must show no readout.
  3. After the Shift+Left loop, wait 2000 ms before `editor-first-beat-zero`.
- done when: `npm run test:gui -- editor_first_beat` passes and the two screenshots were looked at (readout present,
  then gone).

### Task 8. Docs: README and UI guide
- routine: no (edits inside two long files; content checked by reading)
- files: `src/calliope-gui/README.md`, `docs/ui.md` ("Decided by the team" only).
- does:
  - **README.**
    - In the "First beat" paragraph: the keys show the same readout as a drag for a moment. The "Start" field under
      the section toggles takes the offset of the section under the playhead in 1/64 beats (from the found start; for
      later sections from the detected position or the previous section's start), clamped like a drag. Find start
      works in the first section.
    - In the Sections paragraph: Prev / Next go to the start of the previous / next section (like Start while playing).
    - ASCII only.
  - **`docs/ui.md`.**
    - Update the "Sections group (3 rows)" bullet: 4 rows, eight 2.75 rem buttons `+ − ×2 ÷2 Prev |◀ ▶| Next`, row 4
      "Start [field] /64 beat time [Find start]".
    - Update the gui-first-beat bullet: Find start is in row 4 and disabled outside section 1.
    - Update the measured layout bullet with the numbers and positions measured in Task 6.
    - Append a `(gui-first-beat-tuning)` bullet:
      - the keyboard readout (same text and place as a drag, 1.5 s after the last key, gone on blur);
      - the field's commit / Escape / refusal behaviour and message text;
      - Prev / Next and their disabled titles.
- done when: `git diff --stat` shows only those two files; `LC_ALL=C grep -nP '[^\x00-\x7F]' src/calliope-gui/README.md`
  prints nothing new; `npm test` still passes.

## 4. Test strategy

- **Pure logic** (vitest): `section-meta.test.ts` (parse / format / time) and `waveform-geometry.test.ts`
  (`clampMoveSteps` against the Rust clamps' values, `sectionOffset`).
- **Components and state** (vitest + jsdom, mockIPC, fake timers):
  - `WaveformView.test.ts`: the keyboard readout's text, place and lifetime, identical to the drag's text, in both
    modes.
  - `player-state.test.ts`: the payloads of the field and Prev / Next, the refusal, the captured section, and
    "Next = |◀ of the next section".
  - `ControlBar.test.ts`: the row, disabled states, titles, and the field following the target section.
- **Backend**: unchanged and not retested beyond the existing suites. The field and Prev / Next use `move-change` and
  `jump-section`, whose clamps, stale guards and play-while-jumping behaviour are covered by `player_tests.rs`,
  `tempo_map.rs` and `tests/acceptance_player_backend.rs`.
- **GUI on a display**:
  - `gui_player_e2e::player_first_beat_tuning` is the whole flow: the layout log, the readout screenshots, `player.json`
    checks after typed offsets, the jump positions from the logs, and captured audio for Next / |◀ while playing.
  - `gui_editor_e2e::editor_first_beat` covers the Editor readout screenshots.
  - The re-measured constants are validated by rerunning both Player GUI binaries.
  - Screenshots go to `target/gui-shots/` and are looked at by the agent.
- **No hardware, no network**: audio output is the existing capture backend. Repositories are temp copies (`noisy_dirs`
  copies the committed `tests/fixtures/first-beat/noisy-intro.flac`). Dialogs and the edge-AI server are not involved.
- **Whole headless suite**: `cd src/calliope-gui && npm test`. **GUI suite**: `cd src/calliope-gui && npm run test:gui`
  (or `DISPLAY=:1 npm run test:gui:serial`).

## 5. Deployment

Single machine: only the calliope-gui frontend changes. It runs on the laptop (`npm run build:app`). `calliope-stems` is
not touched.

## 6. Manual checks (owner, on the laptop)

1. **Keyboard readout.** Open a track with a found first beat in the Player. Click the waveform in the first section and
   press Right a few times. The readout at the top left of the waveform should show "+N/64 beat (time) from the found
   start" while you press, and disappear about 1.5 s after the last press. Do the same in the Editor's Mix waveform.
2. **Typed offset.** Put the playhead in the first section. In the Sections pane, type `+10` in the "Start" field and
   press Enter. The first bar should move a little later, and the field should read `+10` with its time next to it.
   Play it with Count In and listen to whether the count-in lands better.
3. **Beyond the limit and invalid text.** Type `-9999`: the first bar should stop at 0:00 and the field should show the
   value actually applied. Type `abc`: a message should appear and nothing should move.
4. **Find start.** Press "Find start". The field should go back to `0`.
5. **Other sections.** Put the playhead in a later section (with Next). "Find start" should be greyed out with a hint;
   the field should show that section's offset. Type a small value, for example `-8`, and check that only that bar
   moves.
6. **Prev / Next.** On a track with three or more sections: Next goes to the next section's start, Prev from the middle
   of a section goes to the previous section's start (in section 2 that is the first beat). Next is disabled in the
   last section and Prev in the first.
7. **Prev / Next while playing.** Press Next while playing into a section with Count In on, and compare with pressing
   |◀ in that section while playing. They should behave the same (see open question 1).
8. **Layout.** At 1280x800 with the tree folded, Sections and Loops should still be side by side and nothing should
   need scrolling.

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests |
|---|---|---|
| First bar selected, Right: the readout "+1/64 beat (...) from the found start" at the drag's place, gone about 1.5 s after the last key | 2, 3, 6, 7 | `WaveformView.test.ts` (fake timers: 1499 ms still shown, gone at 1500, refreshed by each key, blur); GUI `player_first_beat_tuning` step 2 and `editor_first_beat` screenshots |
| Detected change moved with the keys shows the same readout as its drag | 3 | `WaveformView.test.ts` Editor-mode key readout equals the drag readout text (with the limit tail) |
| Section 1, `+10` and Enter: first beat 10/64 beat after the found start, field `+10` and its time | 1, 2, 4, 5, 6 | `player-state.test.ts` payload; `ControlBar.test.ts` field `+10` and `+0:00.078`; GUI step 3 (`player.json` 1278.125, `found_ms` 1200) |
| Value beyond the limits: stops at 0:00 / one beat before the next section, field shows the applied value | 2, 4, 5, 6 | `waveform-geometry.test.ts` `clampMoveSteps` (-154 / 167, -166 / 89); `player-state.test.ts` (-9999 → -154, -166); GUI step 4 (start 0.0, screenshot `-154`) |
| Not a whole number: message, nothing changes | 1, 4, 5, 6 | `section-meta.test.ts` `parseOffset`; `player-state.test.ts` (no call, error); `ControlBar.test.ts` (alert, aria-invalid); GUI step 5 |
| First bar moved by keys or Find start: field shows the new offset | 5 | `ControlBar.test.ts` (snapshot after a key move and after `find-start` updates the field); GUI steps 2-3 screenshots |
| Section with a detected change, `-8`: 8/64 before the detected position, others unchanged, field `-8` | 2, 4, 5, 6 | `player-state.test.ts` (index 1, -8); `ControlBar.test.ts`; GUI step 7 (`player.json` 2937.5, others unchanged) |
| Playhead moved from section 1 to 2: field shows section 2's offset | 5, 6 | `ControlBar.test.ts` transport event; GUI step 6 screenshot |
| Playhead in section 2: Find start disabled, field editable | 5, 6 | `ControlBar.test.ts` (disabled + title, field enabled); GUI steps 6-7 |
| Three sections, middle of section 2: Next → start of section 3; Prev → first beat | 4, 5, 6 | `player-state.test.ts` payloads; GUI steps 8-9 (jump positions 1278 and 4200) |
| Last section: Next disabled; first section: Prev disabled | 5, 6 | `ControlBar.test.ts`; GUI step 9 screenshot, step 1 screenshot |
| Playback with Count In on in the target section, Next: counts in as |◀ does | 4, 6 | `player-state.test.ts` (Next payload equals |◀'s for that section); GUI step 10 (Next and |◀ while playing: same transport and audio); existing `jump_section_*` tests in `player_tests.rs` |
| Player at 1280x800: no overflow, Loops beside Sections | 5, 6 | GUI `player_first_beat_tuning` steps 1 and 11, `player_first_beat` step 7, `visual_review` (`overflow=0`, `loops_beside=1`, `sections_w ≤ 496`) |

## 8. Open questions

Answered by the owner (2026-10-26): Prev / Next behave like |◀ while playing, with no count-in (the plan as
written).

1. **What "counts in as |◀ does" means while playing.** Requirement 4.3 says Prev / Next behave like the existing |◀,
   and the plan does exactly that, because they send the same `jump-section` action. Today |◀ **while playing** jumps
   without a gap and plays **no** count-in. It even cancels a running pre-roll (gui-player-tempo-sections: "while
   playing, playback continues without a gap"). Only a Play pressed while paused counts in. The acceptance wording "then
   it counts in as |◀ does" could mean that the owner expects a count-in on Next while playing. The default in the plan:
   identical to |◀ (no count-in while playing; Play after a paused jump counts in). If the owner wants a count-in on a
   jump while playing, that is a backend change (a pre-roll on `Cmd::Jump` into a Count-In section) and should then
   apply to |◀ too.
