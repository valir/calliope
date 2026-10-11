# Plan: Play a backing track (Player view)

Spec: `specs/gui-play-backing-track.md`. Architecture: `docs/architecture.md`. UI guide: `docs/ui.md`.
Path conventions: as in `docs/architecture.md` ("Paths in this document"): `src/*.rs`, `src/ui/...`, `tests/...`,
`build.rs`, `capabilities/`, `package.json` are inside `src/calliope-gui/`.

Revision 2 (2026-10-11): owner answers applied.
- The acceptance criteria are accepted (section 7), with the owner's edit to criterion 8: changing the dropdown
  stops playback.
- Increment A is built now. BPM detection becomes a later spec. The project licence changes to MPL-2.0 (task 18).
- `-`/`+`/`Reset` are disabled at BPM 0.
- `+` creates a loop with no end yet.
- The playback position is saved per track.
- Audio output and MIDI settings belong to a later feature.

The only open question left is the time-stretch library for increment B (section 8). The spec has two requirements
numbered 8; this plan calls the second one **8b** (the control bar sections).

## 1. Summary

The Player view (Alt+5) gets the shared Library tree on the left (Ctrl+L collapses it to a narrow rail) and, on the
right, a player for the selected track. At the top, a dropdown picks what to play (the backing mix or a single stem).
Below it are an empty tablature area, a full-width waveform with the loop sections marked, and a control bar with BPM,
Metronome, Playback and Loops sections. Playback, the metronome tick, the count-in and loop repeats run in Rust, in
one audio stream, so they stay sample-locked. Every setting (source, tempo map, speed, metronome, loops, playback
position) is saved to a `player.json` file next to the track's `track.json` a moment after each change, with no Save
button.

Build order:
- **Increment A (tasks 1-17):** everything except the speed change. `-`, `+`, `Reset` and `Detect` are visible but
  disabled, so the layout is final.
- **Task 18:** the project licence changes from Apache-2.0 to MPL-2.0 (owner decision). It doesn't depend on A and
  can be built with it.
- **Increment B (tasks 19-22): speed change keeping the pitch** (`-`, `+`, `Reset`), once the owner approves a
  time-stretch library (section 8).
- **BPM detection** (`Detect`, several tempos per track, "Beat This!" on archserver) is a **later spec**
  (`gui-play-backing-track-detect.md`, 2.13).

## 2. Design

### 2.1 What is reused, what is new

Reused unchanged:
- `audio_out`: all three backends, and the `e2e-hooks` rule that GUI tests can only use `NullBackend`;
- `transport`: Play/Pause and the 0.3 s scrub-resume rule;
- `stem_audio::decode_i16` and `probe` (FLAC via `claxon`);
- `fsutil::write_atomic`;
- the `TrackBrowser` tree column and the shared `lib` selection;
- the IPC/channel pattern;
- the `Slider`/`Checkbox` components and `time-format.ts`.

New, all in the GUI crate:

| Module | Responsibility | Pure? |
|---|---|---|
| `src/tempo_map.rs` | Tempo segments, beat grid in 1/64-note ticks, ms<->tick, snapping, "set BPM at position", beats in a range | yes |
| `src/player_state.rs` | `player.json` schema (lenient read, strict write), loop list operations (including loops without an end), re-snapping loops after a tempo change, the source-choice rule, load/save of the file | yes (file IO through `fsutil`) |
| `src/player_source.rs` | The dropdown entries of a track; decoding one entry into memory (FLAC: `claxon`; anything else: the external `ffmpeg` through a pipe); waveform peaks | yes except the ffmpeg child |
| `src/metronome.rs` | The click sound (synthesised, our own), count-in plan, placing ticks inside an output block | yes |
| `src/player.rs` (+ `src/player_tests.rs`) | `PlayerManager`/`PlayerSession`, the render `Engine` owned by the audio callback, commands, events, persistence debounce | engine is pure; manager like `editor.rs` |
| `src/player_ipc.rs` | The `#[tauri::command]` wrappers for the Player and the managed `PlayerHost` | Tauri glue |

`src/ipc.rs` is already 1400 lines, so the Player's commands go in their own `src/player_ipc.rs`. They follow every
rule of the IPC pattern: thin wrappers, registered in `gui.rs`, listed in `build.rs` and `capabilities/main.json`. The
static test already strips the `module::` prefix.

### 2.2 The dropdown: sources of a track (spec req. 4)

`player_source::sources(track: &TrackRecord) -> Vec<Source { key, label, file }>`, in this order:

| Key | Label | When |
|---|---|---|
| `backing:<id>` | the variant's `name` (e.g. "Backing") | each entry of `backings` (stem tracks) |
| `audio` | "Backing track" | `audio` is set (plain backing tracks) |
| `original` | "Original mix" | `original` is set (kept at import) |
| `stem:<name>` | stem name with a capital first letter | each entry of `stems` |

- **Missing files:** entries whose file is in the record's `missing` list are left out. A track with no entry is "not
  playable", and the pane shows "<title> has no audio to play."
- **Default** (first visit, or the saved key no longer exists): the first `backing:*`, else `audio`, else the first
  `stem:*` (the spec's rule). `original` is never chosen automatically (owner).
- **Changing the dropdown stops playback** (accepted criterion 8). The position is kept, so Play continues from the
  same place with the new source. The choice is saved at once (2.5) and logged.

**Decoding** (`player_source::decode(file, ffmpeg: Option<&Path>, cancel, progress) -> Pcm {rate, channels, frames,
samples: Vec<i16>}`):
- **`.flac`:** `stem_audio::decode_i16` (16-bit in memory, as in the Editor).
- **Anything else** (e.g. `backing.mp3` of a plain backing track): `ffmpeg -nostdin -hide_banner -loglevel error
  -protocol_whitelist file,pipe -i file:<abs> -map 0:a:0 -f s16le -acodec pcm_s16le -ac 2 -ar 44100 pipe:1`.
  - It runs with `calliope_lib::process` (argv, own process group, PDEATHSIG, cancel) and is read straight into
    memory.
  - No file is written and no MPL decoder is needed (the "no symphonia" rule stands).
  - Without ffmpeg the pane shows "Install ffmpeg to play <file>."
- **Size cap:** the same 1.5 GiB cap as the Editor. One source is at most about 160 MB at 15 minutes.

**Peaks** for the waveform: `peaks(&Pcm, buckets = 2048) -> Vec<f32>`.
- Each value is the maximum `|sample|/32768` over all channels in one of 2048 equal slices of the track (the last
  slice takes the remainder).
- They are computed once per decode and sent in the snapshot (2048 numbers).
- The frontend draws them at any width (2.10).

### 2.3 Tempo map and beat grid (spec req. 14)

```
tempo: { beats_per_bar: 4, segments: [ {start_ms: 500.0, bpm: 120.0}, {start_ms: 10500.0, bpm: 90.0} ] }
```

- **BPM 0 = no segments.** A new track has none:
  - the BPM field shows `0`;
  - loops are in **timestamp mode**;
  - `Metronome On`, `Count In`, `-`, `+` and `Reset` are disabled.

  One or more segments = **BPM mode**.
- **Grid.**
  - A beat is a quarter note. `beats_per_bar` is 4 and stored; time signatures come with the tablatures (owner).
  - The grid unit is a **tick = one 1/64 note = 1/16 beat**, so a 4/4 bar has 64 ticks.
  - Segment *i* has a tick length `d_i = 60000 / (bpm_i * 16)` ms and a tick base `T_i`, where `T_0 = 0` and
    `T_{i+1} = T_i + round((start_{i+1} - start_i) / d_i)`.
  - The first segment's start is the **anchor**: tick 0 = bar 1, beat 1.
  - Before the anchor, the first segment's tempo extends backwards (negative ticks), so one BPM typed at the first
    downbeat covers the whole track.
- **Functions:**
  - `ms_at(tick)`, `tick_near(ms)` (nearest tick, ties away from zero), `bpm_at(ms)`, `segment_at(ms)`;
  - `beats_in(ms_a..ms_b) -> iterator of (ms, accent)`, where accent = the first beat of a bar;
  - `is_multi()` (= more than one segment), which drives the **"V" marker**, a read-only check indicator.
- **Typing a BPM** (`set_bpm_at(position_ms, bpm)`; valid values: 0, or 20.0..=300.0 with one decimal):
  - **No segments, bpm > 0:** one segment starts **exactly at the position**. The user pauses on a downbeat and
    types the tempo, and that anchors the grid.
  - **Otherwise** the position is first **snapped to the nearest tick of the current grid**, so tick bases stay
    integers.
    - If a segment starts there (or the position is before the anchor), that segment's bpm is replaced.
    - Else a new segment starts there.

    It lasts until the next change or the end (spec).
  - **bpm 0** removes the segment in effect at the position. Removing the last one means no tempo (timestamp mode).
  - Neighbouring segments with the same bpm are merged (same grid, nothing audible changes).
  - Typing any value resets the speed to 1.0.
- **Loops follow the audio, not the bar numbers, when the tempo map changes.**
  - Every loop boundary is turned into ms with the old map and back into the nearest tick with the new one.
  - When the new map is empty, boundaries are kept as ms; when the first tempo is entered, ms boundaries become ticks.
  - A missing end stays missing.
  - So a loop stays where the user set it, and its stored kind always matches the mode (spec: "timestamp mode saves
    absolute timestamps, BPM mode only saves the closest time division").
- **Speed** (increment B; 2.12): one factor for the whole track, `speed` (1.0 = original).
  - The BPM field always shows `bpm_at(position) * speed`, so it follows tempo changes while playing.
  - Increment A keeps `speed = 1.0`. At BPM 0 the speed is always 1.0, because clearing the tempo is typing a BPM.

### 2.4 Loops (spec req. 12, 16)

```
loops: [ {id: "loop-1", start: {tick: 64}, end: {tick: 192}, repeat: true, count_in: false},
         {id: "loop-3", start: {tick: 512}, end: null, repeat: true, count_in: false} ], current_loop: "loop-1"
```

- **Storage and order:**
  - A boundary is `{tick: i64}` (BPM mode) or `{ms: f64}` (timestamp mode).
  - **`end` may be `null`**: an *incomplete* loop.
  - Loops are kept **sorted by start**. The current loop is remembered by id, so its number can change when another
    loop is moved before it. At most 99 loops.
- **`+` adds a loop starting at the position** (snapped to the nearest tick in BPM mode) **with no end** (owner). It
  becomes current, with `repeat: true` and `count_in: false`. The user lets the track play and presses `<` at a good
  end point.
- **An incomplete loop** (no end):
  - the `bar2` box is empty, with the placeholder "set with <";
  - the in-loop slider is empty;
  - the waveform draws only its start mark (no bar);
  - loop `Play/Stop`, `Loop On` and `Count-In` are disabled;
  - it never makes playback repeat;
  - `Prev`/`Next` and `-` work on it as on any loop;
  - it is saved as `"end": null` and restored as incomplete.
- **`-`** removes the current loop with no confirmation. The next one (or the previous, if it was the last) becomes
  current.
- **`Prev`/`Next`** select the previous/next loop and **jump the position to its start at once** (no 0.3 s delay).
  Playing continues from there.
- **`<` sets the end** to the position (snapped in BPM mode, forward or backward, whichever is closer):
  - if that is after the start, the loop gets that end (this also completes an incomplete loop);
  - if it is at or before the start, nothing changes and the Loops section shows "The end must be after the start"
    in its status line (inline, no pop-up).
- **`>` sets the start** to the position (snapped likewise):
  - incomplete loop: the start moves;
  - complete loop, new start before the end: the start moves;
  - **complete loop, new start at or after the end: the start moves there and the end is cleared**. The loop becomes
    incomplete again and waits for a new `<`. This fits the "mark start, play, mark end" workflow, and no loop is ever
    silently reversed.
- **The `bar1`/`bar2` boxes** show the start/end and are editable (Enter commits, Escape reverts):
  - in BPM mode as `bar.beat.tick`, e.g. `5.1.0` or `12.3.8` (bar from 1, beat 1..beats_per_bar, tick 0..15); bars
    before the anchor are 0 or negative;
  - in timestamp mode as `M:SS.mmm`, e.g. `1:07.250`; input also accepts the time field's forms;
  - typing into `bar2` can complete an incomplete loop;
  - a typed start at or after the end, or a typed end at or before the start, marks the field invalid and changes
    nothing (typing is deliberate, unlike `>`).
- **The section box** is read-only and shows `2/5` (current / count), or `-` when there are no loops.
- **The small slider** is read-only (`aria-readonly`, not focusable). It shows the position inside the current
  complete loop, 0..100 %, and is empty when the position is outside the loop or the loop is incomplete.
- **`Loop On`** (`repeat`, per loop): while it is on, **any playback that reaches the current loop's end while inside
  the loop jumps back to its start**. This includes main Play (the usual A-B repeat).
- **`Count-In`** (per loop) plays a count-in before every pass of that loop (2.6).
- **Loop `Play`** jumps to the loop start and plays (with a count-in, 2.6).
  - It reads `Stop` while that loop is playing.
  - With `Loop On` off, it plays the section once and stops at the end; the position returns to the loop start.
  - `Stop` stops and returns to the loop start.
  - There is no loop pause (spec), but the main Pause still pauses anything.
- **Speed changes** (increment B) apply to loop playback automatically (one stream).

### 2.5 Persistence: `player.json` next to `track.json` (spec req. 3, 4, 13; owner-confirmed)

```json
{
  "schema_version": 1,
  "source": "stem:guitar",
  "position_ms": 83400,
  "tempo": {"beats_per_bar": 4, "segments": [{"start_ms": 500.0, "bpm": 120.0}]},
  "speed": 1.0,
  "metronome": {"on": false, "count_in": true},
  "loops": [{"id": "loop-1", "start": {"tick": 64}, "end": null, "repeat": true, "count_in": false}],
  "current_loop": "loop-1"
}
```

- **Where:** `<root>/tracks/<id>/player.json`, the "next to the track" file of req. 4.
  - Writing `track.json` after every adjustment would change its revision each time, so a Library edit in progress
    would hit a save conflict.
  - `player.json` is reserved as a file name like `track.json` (`fsutil::validate_file_name`).
  - **Export** copies it when present. Delete moves the whole folder to the trash as before. Library scans ignore
    it.
- **One state per track, shared by all sources** (owner): the stems and backings of a track are sample-aligned, so
  the tempo map, loops, speed, metronome and position apply to whichever source is selected. `source` records the
  choice.
- **When it is written:** each change marks the session dirty, and the ticker writes the file (`write_atomic`) 500 ms
  after the last change. It is written at once on track switch, `close_player`, view leave and app exit. "Persisted as
  soon as adjusted" is checked as "on disk within 1 s".
- **Playback position** (owner): `position_ms` (integer ms) is saved:
  - with every write;
  - after pause, seek, skip, Prev/Next, loop Stop, the end of the track (saved as 0), a source change, view leave,
    track switch, close and exit;
  - while playing, every 10 s (so a crash loses little), not on every position event.

  It is restored when the track is opened again or after a restart. A value past the end gives `0:00.0`.
- **Never creates folders.** If the track folder is gone (deleted meanwhile), the write fails, the pane shows "Could
  not save the player settings: ..." inline, and the session keeps working.
- **Lenient read:**
  - a missing file means defaults;
  - an unreadable or invalid file means defaults plus an inline note: "Player settings in player.json could not be
    read; they will be replaced when you change something";
  - bad parts are dropped one by one: a bad loop is skipped, a bad end becomes `null`, an unknown source key falls
    back to the default, a bpm out of range drops that segment, and `speed` and `position_ms` are clamped;
  - unknown fields are kept on write;
  - a newer `schema_version` is read-only: the Player plays with defaults and never overwrites the file (same rule as
    `track.json`).
- **Not saved:** the playing state and the tree collapse.

### 2.6 Metronome and count-in (spec req. 10, 15)

- **One stream.** Clicks are mixed by the same render callback that plays the track, after the (future) speed change.
  So a tick is sample-aligned with the music whatever the device latency. The output is hard-clipped at full scale,
  like the Editor mix.
- **Sound** (`metronome::Click`): our own synthesis, no samples from anywhere.
  - A 2 kHz sine on the first beat of a bar (accent), 1.5 kHz on the other beats.
  - Exponential decay over 30 ms, peak 0.5 (-6 dBFS). No volume control in this feature.
  - Rendered into a pre-allocated buffer at the output rate when a session opens.
- **Metronome On** (needs BPM mode): a click on every beat of the grid (`tempo_map::beats_in`) inside the played
  range, mapped to output frames.
- **Count-in** (`Count In`, metronome section; needs BPM mode):
  - `count_in_plan(bpm_at(start) * speed, rate)` gives three audible clicks one beat apart. Playback then starts on
    the fourth beat, which is silent (spec): clicks at output frames 0, B and 2B, audio from 3B, where B is one beat.
  - During it the position does not move and the transport reports `counting_in: true`. Pause cancels it.
- **When a count-in plays:**
  - main Play from paused/stopped, if `Count In` is on (not on a resume after a scrub);
  - loop Play, if `Count In` is on **or** that loop's `Count-In` is on;
  - each repeat of a loop (the jump back to its start), only if that loop's `Count-In` is on.
- Count-in and the metronome are independent: the count-in plays even with the metronome off.

### 2.7 The engine (Rust, real-time)

The Player is a second, independent engine next to the Editor's (`player.rs`). It adds count-ins, loop repeats and
clicks that must happen at an exact sample without waiting for the 20 ms ticker. Its playback state lives **inside
the audio callback**:

- **`Engine`** is a pure struct, unit-tested without any backend. It holds:
  - the decoded source (`Arc<Pcm>`) and the position (frames);
  - the run state `Stopped | Paused | CountIn { left, next_click } | Playing`, and the `audible` gate of the scrub
    rule;
  - the current complete loop (`start` and `end` frames, `repeat`, `count_in_each`; none for an incomplete loop) and a
    one-shot "stop at loop end";
  - the beat grid (`Arc<Vec<(frame, accent)>>`), metronome on, and the click voice;
  - (increment B) the stretcher.

  `fn apply(&mut self, cmd: Cmd)` and `fn render(&mut self, out: &mut [f32])`. It never allocates or locks.
- **Commands in.** The callback closure owns the `Engine` and a `std::sync::mpsc::Receiver<Cmd>` from a bounded
  `sync_channel(64)` (the array flavour, so no allocation per message). It drains the commands, then renders. Replaced
  `Arc`s (an old grid) go back through a second bounded channel, so they are freed on the ticker thread, not in the
  callback.
- **Status out** of the callback is atomics: position, run state, `loop_playing`, a wrap counter and an "ended" flag.
- **Fades.** Discontinuities (seek, loop repeat, start after a count-in) get a 3 ms linear fade-in. A known jump point
  (loop end, one-shot stop) gets a 3 ms fade-out before it, so loops repeat without clicks. Everything else is an
  exact copy of the source (speed 1.0), which the tests compare sample by sample.
- **`PlayerManager`** (Tauri-managed behind a `Mutex` in `PlayerHost`, the same shape as `EditorManager`/`EditorState`)
  holds:
  - the session: track id, dir, sources, state, tempo map, `Arc<Pcm>`, peaks, `Transport`, output handle and command
    sender;
  - the deferred-drop trash for output handles;
  - the dirty flag and the periodic position save.

  Opening and source decoding run without the manager lock (ticket + generation, as in the Editor). The device is
  opened outside the lock (`OutputRequest` pattern). When no output is open, the manager's own copy of position and
  state is authoritative; opening the output builds the `Engine` from it.
- **Source change** (`select_source`): stop playback (keep the position), close the output, decode the new source off
  the lock, then install it. The next Play opens the output at the new source's sample rate (an mp3 decoded at
  44.1 kHz can sit next to 22.05 kHz stems).
- **Ticker:** the existing 20 ms thread in `gui.rs` also calls `PlayerHost::tick(now)`. It:
  - applies a due scrub resume (`Cmd::Resume`);
  - turns the callback's flags into state (loop one-shot end; end of track = stop at `0:00.0`);
  - reports device errors;
  - writes a dirty `player.json`;
  - emits `transport` events: on a change, else every 100 ms while playing or counting in.
- **One sound at a time:** Play in the Player stops the Editor, and Editor Play pauses the Player, through the IPC
  layer, so two streams never play at once. The views already stop/pause on leave; this is the safety net.
- **Shared backend:** both managers share one `Arc<dyn OutputBackend>` built by `select_backend()` in `gui.rs`
  (`EditorManager::new` takes the `Arc`), so the GUI tests' capture file is one WAV.

### 2.8 Behaviour

- **Player pane states:**
  - no selection: "Select a track to play it.";
  - not playable: "<title> has no audio to play.";
  - loading: "Loading..." with progress;
  - error: an inline alert;
  - active.

  Inactive states show the layout with disabled controls.
- **Selection** is the shared `lib.selectedId`: selecting a track in the Player selects it in Library and Editor, and
  the other way round. A Library edit in progress locks the tree (existing behaviour).
- **Track change while playing:** the session is closed (state flushed, position saved), and the new track opens
  paused at its saved position.
- **Leaving the Player view:** `player_leave` pauses (position kept), closes the output and flushes the state. Coming
  back shows the same track paused at the same position.
- **Playback section:**
  - `Play/Pause` reads `Pause` while playing or counting in.
  - The position slider runs 0..duration in 100 ms steps, with the 0.3 s scrub rule while playing.
  - `Rew`/`FF` jump -5 s/+5 s at once, clamped.
  - The time field uses `M:SS.T`, the Editor's format and parser. The wheel moves -/+0.1 s and Enter commits. Out of
    range is refused with "Enter a time between 0:00.0 and <duration>".
  - The end of the track stops at `0:00.0` (like the Editor).
- **BPM section:**
  - The BPM field, labelled "BPM", shows the BPM at the position, or `0`.
    - While it is focused and edited, transport events don't overwrite it.
    - Enter commits `set_bpm_at(position)`; Escape reverts.
    - Invalid input shows "Enter a BPM between 20 and 300, or 0 to clear".
  - `-`, `+` and `Reset` are disabled at BPM 0 (owner) and, in increment A, always disabled, with the hint "Comes in a
    later update".
  - `Detect` is disabled with the hint "Comes in a later update".
  - The "V" indicator is checked when the track has several tempos.
- **Metronome section:** `On/Off` and `Count In` push buttons (`aria-pressed`, amber when on), disabled without a
  tempo.
- **Loops section:**
  - three groups, as in req. 12: (`Prev`, `+`, section box, `-`, `Next`), (`>`, `bar1`, small slider, `bar2`, `<`),
    (`Play/Stop`, `Loop On`, `Count-In`);
  - buttons that need a current loop are disabled without one, and the third group also while the loop is incomplete;
  - a one-line status area shows refusals such as "The end must be after the start".
- **Keyboard** (the UI guide's list):
  - These keys work in the Player when the focus is not in a text field, slider, button, select or the tree (like the
    Editor's Space rule): `Space` Play/Pause, `b` = `>`, `e` = `<`, `←`/`→` = Prev/Next, `L` = Loop On.
  - `[`/`]` = `-`/`+` (increment B; the spec's 5 BPM step, not the guide's 5 %).
  - `Ctrl+L` collapses the tree, wherever the focus is in the Player view.
- **Ctrl+L collapse:**
  - The tree column (20 rem) becomes a 3.5 rem rail with a single large "Library" icon button (tooltip "Show the
    library (Ctrl+L)") that expands it again.
  - The `h1#view-heading` "Player" stays in the rail (visually hidden) for focus.
  - Not remembered between runs (like `Ctrl+B`); Player view only.
- **Layout:**
  - The right pane has the dropdown row (label "Play:", the select, the track title).
  - The tablature area takes the remaining height, as a dashed placeholder: "The tablature will be shown here
    (gui-tablatures)."
  - Below it, pinned, are the waveform lane (about 6 rem high, full width) and the control bar.
  - The bar's four sections are bordered groups in a wrapping row. At 1280x800 they fit in two rows (BPM + Metronome +
    Playback, then Loops); at 1024x640 with the tree collapsed, the same. Nothing scrolls out of view.
  - Control sizes follow the guide (buttons at least ~50 px tall). The visual review task fixes the final details.
- **Select component:** shadcn-svelte `select` (bits-ui `Select`), copied into `src/ui/lib/components/ui/select/` like
  the other components (MIT, recorded in `docs/licences.md`).
- **Settings placeholders:** "Audio output" and "MIDI interface" (and the Settings view's feature list) name the later
  feature `gui-audio-midi-settings` instead of this one (owner). The owner may add it to the roadmap.

### 2.9 IPC (pattern of `docs/architecture.md`)

Rust types live in `src/player.rs`, commands in `src/player_ipc.rs`, and TS mirrors in `src/ui/lib/ipc.ts`. Every
command takes the track `id` and fails with "no player session for <id>" when it isn't the open one.

```ts
type Boundary = { tick: number } | { ms: number };
interface PlayerLoop { id: string; start: Boundary; end: Boundary | null; start_ms: number; end_ms: number | null;
  repeat: boolean; count_in: boolean }
interface PlayerTransport {
  playing: boolean /* playing, counting in or resume pending */; counting_in: boolean; resume_pending: boolean;
  position_ms: number; loop_playing: boolean; bpm: number /* shown: bpm_at(position) * speed, 0 = none */;
  tempo_index: number | null
}
interface PlayerSnapshot {
  id: string; title: string; sources: { key: string; label: string }[]; source: string;
  duration_ms: number; sample_rate: number; peaks: number[] /* 2048 values 0..1 */;
  mode: 'bpm' | 'time'; tempo: { beats_per_bar: number; segments: { start_ms: number; bpm: number }[]; multi: boolean };
  speed: number; metronome: { on: boolean; count_in: boolean };
  loops: PlayerLoop[]; current_loop: number | null /* index */; loop_message: string | null;
  transport: PlayerTransport; can_change_speed: boolean; note: string | null /* settings file note */
}
type PlayerEvent =                       // tagged by "kind"
  | { kind: 'loading'; id: string; progress: number }
  | ({ kind: 'transport'; id: string } & PlayerTransport)
  | { kind: 'audio-error'; id: string; message: string }       // playback stopped
  | { kind: 'state-error'; id: string; message: string };      // player.json could not be written
type TempoAction = { kind: 'set-bpm'; bpm: number } | { kind: 'speed-step'; steps: number } | { kind: 'speed-reset' };
type LoopAction =
  | { kind: 'add' } | { kind: 'remove' } | { kind: 'select'; delta: -1 | 1 }
  | { kind: 'mark'; edge: 'start' | 'end' }                       // > and <
  | { kind: 'set'; edge: 'start' | 'end'; value: Boundary }       // typed bar1/bar2
  | { kind: 'flags'; repeat: boolean; count_in: boolean }
  | { kind: 'play' } | { kind: 'stop' };
```

`loop_message` carries the refusal of the last `mark` ("The end must be after the start"), or null.

| Command | Returns | Notes |
|---|---|---|
| `open_player(id, events: Channel<PlayerEvent>)` | `PlayerSnapshot` | async + spawn_blocking; closes (and flushes) the previous session; restores the saved position; a superseded open fails with `superseded` |
| `close_player()` | `()` | flushes `player.json` |
| `get_player()` / `watch_player(events)` | `PlayerSnapshot \| null` | re-attach after a webview reload |
| `player_select_source(id, source)` | `PlayerSnapshot` | stops playback, keeps the position, decodes off the lock |
| `player_play(id)`, `player_pause(id)` | `PlayerTransport` | Play applies the count-in rule |
| `player_seek(id, position)`, `player_nudge(id, delta)` | `PlayerTransport` | ms; clamped; 0.3 s scrub rule while playing |
| `player_skip(id, delta)` | `PlayerTransport` | Rew/FF; instant jump, clamped |
| `player_leave(id)` | `PlayerTransport` | pause, close the output, flush |
| `player_tempo(id, action: TempoAction)` | `PlayerSnapshot` | speed kinds fail with "Speed change is not available yet" in increment A, and with "Enter a BPM first" at BPM 0 |
| `player_metronome(id, on, count_in)` | `PlayerSnapshot` | refused without a tempo |
| `player_loop(id, action: LoopAction)` | `PlayerSnapshot` | `play`/`flags` refused on an incomplete loop ("Set the loop end first") |

That is fourteen new commands, each in `generate_handler!`, `COMMANDS` in `build.rs` and `capabilities/main.json`.
Detect's commands come with the later spec.

**Log lines** (for the GUI tests):
- Frontend (`calliope-ui: `):
  - `player active id=<id> source=<key> sources=<n> mode=<bpm|time> loops=<n> position=<ms>`
  - `player inactive id=<id|none>`
  - `player source key=<key>`
  - `player play|pause position=<ms>`
  - `player seek position=<ms>`
  - `player skip delta=<ms>`
  - `player bpm value=<v> position=<ms>`
  - `player metronome on=<b> count_in=<b>`
  - `player loop <action> index=<n|none> count=<n> complete=<b>`
  - `player tree collapsed=<b>`
  - `player leave pause`
  - `error player <command>: <msg>`
- Rust (`calliope: `):
  - `player open id=<id> source=<key> duration_ms=<ms> rate=<hz> position_ms=<ms>`
  - `player transport playing=<b> counting_in=<b> loop=<n|none> position_ms=<ms>` (changes only)
  - `player count-in clicks=3 beat_ms=<ms>`
  - `player wrap loop=<n> count_in=<b>`
  - `player state saved id=<id>`
  - `player state error id=<id> message=<m>`

### 2.10 Frontend modules

| File | Content |
|---|---|
| `src/ui/lib/bar-format.ts` | `formatBar(tick, beatsPerBar)`, `parseBar(text, beatsPerBar) -> number \| null`, `formatLoopTime(ms)` (`M:SS.mmm`), `parseLoopTime(text) -> number \| null`; pure |
| `src/ui/lib/bpm-format.ts` | `formatBpm(bpm)` (`0`, `120`, `92.5`), `parseBpm(text) -> number \| null` (0 or 20..300, one decimal); pure |
| `src/ui/lib/waveform-geometry.ts` | `columns(peaks, width) -> number[]` (max of the peaks under each pixel column), `xOf(ms, durationMs, width)`, `loopMarks(loops, durationMs, width)` (`x2` null for an incomplete loop); pure |
| `src/ui/lib/player-state.svelte.ts` | Module `$state` `pl` (status, snapshot fields, transport, messages, `treeCollapsed`) and the actions (`syncSelection`, `leavePlayer`, `selectSource`, `togglePlay`, `seek` (coalesced), `nudge`, `skip`, `setBpm`, `setMetronome`, `loop(action)`, `attachPlayer`), the event handler, stale-id guard, log lines |
| `src/ui/lib/components/ui/select/` | shadcn-svelte select over bits-ui, large size, both themes, CSP-safe |
| `src/ui/components/editor/TimeField.svelte` | generalised with props (`positionMs`, `durationMs`, `active`, `onseek`, `onnudge`) and used by both views |
| `src/ui/components/player/PlayerPane.svelte` | states, dropdown row, tablature placeholder, waveform lane, control bar |
| `src/ui/components/player/{SourceSelect,WaveformView,BpmSection,MetronomeSection,PlaybackSection,LoopsSection}.svelte` | the parts; see `WaveformView` below |
| `src/ui/views/PlayerView.svelte` | `TrackBrowser` (or the collapsed rail) + `PlayerPane`; keys |

`WaveformView` draws on a `<canvas>` (scripted drawing is CSP-safe) and redraws on `ResizeObserver` and on changes:
- amber loop marks: 2 px lines at start and end, and a 4 px bar joining them along the top;
- an incomplete loop gets its start line only;
- the current loop is brighter;
- a playhead line.

### 2.11 Choices inside the answers

- BPM input: 0 or 20..300 with one decimal; the field shows integers without decimals.
- Max 99 loops. A new loop has `repeat: true`, `count_in: false`.
- Loop marks and the playhead are drawn by the frontend from ms values in the snapshot/events (no time kept in JS).
- Peaks: 2048 max-abs values per source. Fades: 3 ms at jumps.
- The `transport` event interval stays 100 ms. That is smooth enough for the waveform playhead; the tablature cursor
  in `gui-tablatures` may need more.

### 2.12 Increment B: speed change keeping the pitch (tasks 19-22)

- **`src/stretch.rs`:** `trait Stretcher { fn reset(); fn prime(&mut self, pre_roll: &[f32]); fn process(&mut self, input:
  &[f32], output: &mut [f32]); fn latency_frames() -> usize }`, implemented with the library the owner approves
  (section 8). At speed 1.0 the engine bypasses it (the bit-exact path of increment A).
- **Engine:**
  - For each output block of N frames it feeds `round(N * speed)` source frames (fractional remainder carried), so the
    position advances at `speed`.
  - The reported position subtracts the stretcher latency, scaled by speed.
  - Seeks and loop repeats call `prime` with the 100 ms before the target.
  - Clicks are placed at `(beat - pos) / speed`. The count-in uses `bpm * speed`.
- **Buttons** (only in BPM mode; disabled at BPM 0, owner):
  - `-`/`+`: target = shown BPM -/+ 5, speed = target / `bpm_at(position)`, clamped to 0.50..1.50;
  - `Reset`: speed = 1.0, so the field shows the correct BPM at the position;
  - typing a BPM resets the speed to 1.0;
  - the speed is saved in `player.json`.

### 2.13 Later spec: BPM detection

`Detect` stays disabled here. The owner chose "Beat This!" (CPJKU) on archserver for it, in a separate spec
(`gui-play-backing-track-detect.md`). Plan sketch for that spec:
- additive `calliope-stems` endpoints `/v1/beats`, with a `--beat-tracker` adapter and a stub tracker in tests;
- the app turns beats and downbeats into tempo segments, which replace the map through `set_bpm`-style logic (loops
  re-snapped, speed reset).

This plan's tempo map, `player_tempo` action enum and `can_change_speed`-style flags leave room for it.

### 2.14 Project licence: Apache-2.0 to MPL-2.0 (task 18, owner decision)

The owner switches all of Calliope (three crates and the npm project) to MPL-2.0. MPL-2.0 is file-level copyleft;
the Apache-2.0 and MIT dependencies stay compatible.
- **`LICENSE`** becomes the exact official MPL-2.0 text. The source is `https://www.mozilla.org/media/MPL/2.0/index.txt`;
  `licence-templates/MPL-2.0.txt` must be byte-identical, and the task checks both against it.
- **Licence fields:** `package.license` in the root `Cargo.toml`, `license` in `src/calliope-gui/package.json` and in
  the root entries of `package-lock.json` become `MPL-2.0`.
- **The notices generator** (`third-party-notices.ts`) read the Apache-2.0 standard text from `LICENSE`. That text
  moves to `licence-templates/Apache-2.0.txt`, and the generator reads it from there. The header line becomes
  "Calliope is licensed under the Mozilla Public License 2.0." `src/calliope-stems/THIRD-PARTY-NOTICES.txt` is
  regenerated (`npm run notices:stems`).
- **Notice for recipients:** the README's licence section is updated, with the MPL Exhibit A notice. No per-file
  headers: MPL allows the notice "in a location (such as a LICENSE file in a relevant directory) where a recipient
  would be likely to look".
- **`docs/licences.md`:**
  - Calliope's own licence becomes MPL-2.0;
  - the MPL-crate paragraph is reworded;
  - the `symphonia` sentence records that it was rejected for its MPL-2.0 licence and that the owner can now
    reconsider. The rule is not lifted.

## 3. Tasks

Every task ends with the headless suite green: `cd src/calliope-gui && npm test`. GUI tasks also run
`cd src/calliope-gui && DISPLAY=:1 npm run test:gui`.
- Tasks 1-17 are increment A.
- Task 18 is the licence switch; it is independent of A and can be built with it.
- Tasks 19-22 (increment B) wait for the owner's approval of the library in section 8.

### Increment A

### Task 1: Player test fixtures
- routine: no
- files: `src/calliope-gui/tests/fixtures/player/make-fixtures.sh` (new), `src/calliope-gui/tests/fixtures/library-player/**`
  (generated, committed), `src/calliope-gui/tests/fixtures/import/README.md` (one line pointing to the script)
- does: a bash script in the style of `tests/fixtures/editor/make-fixtures.sh` (same `ff()` helper, bitexact flags,
  idempotent, `FORCE=1`, deterministic `track.json`).
  - **Repository and audio:** `library-player/`, with the marker `{"schema_version": 1}` and the album "Player Tests".
    Audio is 22050 Hz s16 FLAC, 20 s each unless stated.
  - **`0199c0a0-0000-7000-8000-000000000301` "Practice Song":** type stem.
    - Stems: vocals (440 Hz mono), guitar (660 Hz mono).
    - `backings/backing.flac`: 330 Hz stereo with an amplitude envelope (0-5 s 0.2, 5-10 s 0.9, 10-15 s 0.5,
      15-20 s 0.9, via the lavfi `volume` filter with `eval=frame`). It is listed as variant `backing`/"Backing",
      with the fields as in the Editor fixture "Mixed Before".
    - `original.flac`: 550 Hz stereo, with `"original": "original.flac"`.
  - **`...0302` "Stems Only":** type stem; stems bass (110 Hz mono) and drums (220 Hz mono); no backings.
  - **`...0303` "MP3 Backing":** type backing; `audio` "backing.mp3" = 330 Hz stereo 44.1 kHz MP3 (`-c:a libmp3lame -b:a 128k`).
  - **`...0304` "With Tempo":** the same stems and backing as "Practice Song" (copies), plus this `player.json`:
    `{"schema_version": 1, "source": "stem:guitar", "position_ms": 4000, "tempo": {"beats_per_bar": 4, "segments": [{"start_ms": 500.0, "bpm": 120.0}, {"start_ms": 10500.0, "bpm": 90.0}]}, "speed": 1.0, "metronome": {"on": false, "count_in": false}, "loops": [{"id": "loop-1", "start": {"tick": 64}, "end": {"tick": 192}, "repeat": true, "count_in": false}, {"id": "loop-2", "start": {"tick": 384}, "end": {"tick": 448}, "repeat": false, "count_in": true}, {"id": "loop-3", "start": {"tick": 512}, "end": null, "repeat": true, "count_in": false}], "current_loop": "loop-1"}`.
    The loops are loop-1 = 2500..6500 ms, loop-2 = 13166.667..15833.333 ms, and loop-3 = from 18500 ms, incomplete.
  - **`...0305` "Missing Backing":** type stem; stem vocals (440 Hz mono); a `backings` entry `backing` whose file is
    absent.
  - **`...0306` "Missing Audio":** type backing; `audio` "gone.flac", file absent (a track with nothing to play).
- done when:
  - the script succeeds twice and the second run changes nothing (`git status --porcelain` is clean after committing
    the first);
  - `ffprobe` confirms the rates, channels and durations;
  - every JSON file parses (`python3 -m json.tool`);
  - `npm test` passes.

### Task 2: Reserve the file name `player.json`
- routine: yes
- files: `src/calliope-gui/src/fsutil.rs`
- does:
  - In `validate_file_name`, directly after the `if name == "track.json" { ... }` block, add
    `if name == "player.json" { return Err("\"player.json\" is reserved".into()); }`.
  - In the test that loops over bad names (`for bad in ["..", ".", "a/b", ".x", "track.json", ...]`), add
    `"player.json"` to the array.
  - Next to `assert!(validate_file_name("track.json.bak").is_ok());` add
    `assert!(validate_file_name("player.json.bak").is_ok());`.
- done when: `cargo test -p calliope-gui fsutil` passes and `npm test` passes.

### Task 3: Tempo map (`tempo_map.rs`)
- routine: no
- files: `src/tempo_map.rs` (new), `src/main.rs` (mod)
- does: everything in 2.3:
  - `TempoMap { beats_per_bar, segments }`, `TICKS_PER_BEAT = 16`, tick bases;
  - `ms_at`, `tick_near`, `bpm_at`, `segment_at`, `beats_in`, `is_multi`;
  - `set_bpm_at` (anchor, snap, replace/insert/remove, merge) and `validate_bpm`;
  - `resnap(boundary, old, new) -> Boundary` for loop boundaries.
- done when: the module tests pass, at least:
  - **One segment, 120 BPM at 500 ms:** `ms_at(64) == 2500`, `tick_near(2510) == 64`, `tick_near(2515.7) == 65`,
    `ms_at(-16) == 0`.
  - **The "With Tempo" map:** `T_1 == 320`, `ms_at(384)` within 1e-6 of 13166.667, `ms_at(512)` within 1e-6 of 18500,
    `bpm_at(10499) == 120`, `bpm_at(10500) == 90`.
  - **Beats:** `beats_in(250..2000)` = 500, 1000, 1500, with the accent on 500 only.
  - **`set_bpm_at`:**
    - empty map + 120 at 1234 gives one segment at exactly 1234;
    - the 120 grid + 90 at 10510 gives a segment at 10500 (snapped);
    - 100 typed at 5000, with a change at 10500, gives a new segment 5000..10500 only;
    - 0 at 12000 removes the 90 segment;
    - 0 on the only segment gives an empty map;
    - equal neighbours merge;
    - 19.9 and 300.1 are refused.
  - **`resnap`:** keeps a boundary's time within half a tick, and converts tick <-> ms when the map becomes empty /
    non-empty.

### Task 4: Player state and `player.json` (`player_state.rs`), export
- routine: no
- files: `src/player_state.rs` (new), `src/main.rs`, `src/repository.rs` (export copies `player.json` when present),
  `src/repository_tests.rs`
- does: 2.4 and 2.5:
  - types with serde, including `end: Option<Boundary>` and `position_ms`;
  - `load(dir) -> (PlayerState, Option<note>, writable)`: lenient, unknown fields kept, a newer schema read-only;
  - `save(dir, &state)` via `fsutil::write_atomic`, refusing to create the folder;
  - the loop operations `add` (no end), `remove`, `select`, `mark` (with the `<`/`>` rules of 2.4 and their refusal
    message), `set` (typed values, with invalid cases refused) and `flags` (refused on an incomplete loop);
  - sorting by start, keeping the current id; 99 max;
  - `apply_tempo_change(new_map)` re-snapping every loop;
  - `choose_source(saved, sources)` (the default rule of 2.2).
- done when: the module tests pass, at least:
  - **Round trip:** the "With Tempo" file round-trips, unknown fields kept, `loop-3` still with `end: null`, and
    `position_ms` 4000.
  - **Lenient read:** invalid JSON gives defaults plus a note; a bad loop is dropped and the rest kept; a malformed
    `end` becomes `null`.
  - **Read-only and folders:** `schema_version: 2` makes the state read-only (save refused, file unchanged); a save
    into a deleted folder fails and creates nothing.
  - **Loop rules, BPM mode, 120 at 500:**
    - `+` at 3000 ms gives start tick 80 and `end: null`;
    - `flags` on it is refused;
    - `<` at 9000 gives end tick 272;
    - `<` at 2000 (before the start) changes nothing and returns "The end must be after the start";
    - `>` at 10000 (after the end) gives start tick 304 and `end: null`;
    - `>` at 4000 on a complete loop 80..272 moves only the start;
    - typed `set` start at or after the end is refused.
  - **Order:** loops stay sorted and the current id survives.
  - **Source choice:** `choose_source` picks `backing:backing`, then `audio`, then the first stem, never `original`;
    a stale saved key falls back.
  - **Export:** a repository test shows Export copies `player.json` next to `track.json`, and still works without it.

### Task 5: Sources, decoding and peaks (`player_source.rs`)
- routine: no
- files: `src/player_source.rs` (new), `src/main.rs`
- does: 2.2:
  - `sources(&TrackRecord)`;
  - `decode`: FLAC via `stem_audio`, else an ffmpeg pipe through `calliope_lib::process` with cancel; messages for a
    missing ffmpeg and for a failed decode, naming the file;
  - `peaks(&Pcm, 2048)`.
- done when: the module tests pass on copies of `library-player`:
  - "Practice Song" lists `backing:backing`, `original`, `stem:vocals` and `stem:guitar`, labelled "Backing",
    "Original mix", "Vocals" and "Guitar";
  - "Missing Backing" lists only `stem:vocals`, and "Missing Audio" lists nothing;
  - "MP3 Backing" decodes through the real local ffmpeg to 44100 Hz stereo, about 20 s (+/- 0.1 s);
  - a FLAC decodes to exactly the samples `stem_audio::decode_i16` gives;
  - the peaks of the "Practice Song" backing are about 0.2 in the first quarter and about 0.9 in the second (+/- 0.02);
  - a cancelled ffmpeg decode returns promptly with an error and leaves no child process.

### Task 6: Metronome click and scheduling (`metronome.rs`)
- routine: no
- files: `src/metronome.rs` (new), `src/main.rs`
- does: 2.6:
  - `Click::new(rate)` (two pre-rendered sounds);
  - `count_in_plan(bpm, rate) -> CountIn {clicks: [u64; 3], start: u64}`;
  - `place(beats: &[(u64, bool)], block_start, block_len, speed) -> iterator of (offset, accent)`.
- done when: the module tests pass:
  - at 22050 Hz and 120 BPM the count-in clicks are at 0, 11025 and 22050, and the audio starts at 33075;
  - the click peak is 0.5 and has decayed below 0.01 after 30 ms;
  - accent and normal clicks differ in dominant frequency (2 kHz / 1.5 kHz, checked with a Goertzel filter in the
    test);
  - `place` returns only the beats inside a block, at exact offsets at speed 1.0.

### Task 7: Player engine core (`player.rs`): open, play, seek, source switch, persistence
- routine: no
- files: `src/player.rs` (new), `src/player_tests.rs` (new, `#[cfg(test)] #[path]`), `src/main.rs`
- does: 2.7 without the metronome, count-in and loops:
  - `Engine` (`apply`/`render`, fades, ended flag), the command and return channels, and the status atomics;
  - `PlayerManager`: the ticketed open off the lock (restores `position_ms`), the `OutputRequest` pattern and
    deferred drops;
  - `play`, `pause`, `seek`/`nudge` with the 0.3 s rule, `skip`, `select_source` (stop, keep the position, close,
    decode, install), `leave`;
  - `tick(now)` with the events, the 500 ms debounced save and the 10 s position save while playing; `close` flushing;
  - the types of 2.9 and the Rust log lines.
- done when: the tests pass with `ManualBackend` on copies of `library-player`:
  - **Open:** "Practice Song" gives 4 sources, `source == backing:backing`, 20000 ms and position 0; "With Tempo"
    opens at 4000 ms with `stem:guitar`.
  - **Play:** after `play`, the pulled frames equal the decoded backing samples exactly (outside the 3 ms fade-in).
  - **Pause:** zeros and a frozen position, and `position_ms` is on disk within 500 ms.
  - **Seek and skip:** `seek` while playing is silent until `tick(+300ms)`; `skip(-5000)` at 2 s gives 0.
  - **Source change:** `select_source("stem:guitar")` while playing at 4 s stops playback (`playing: false`, the
    output closed) and keeps 4000 ms; the next `play` gives guitar samples from 4 s.
  - **End of track:** stops at 0 and saves 0.
  - **Debounce:** a change marks the session dirty, and `player.json` appears after `tick(+500ms)`, not at `+499ms`.
  - **Periodic save:** while playing, `position_ms` on disk is updated after `tick(+10s)`.
  - **Restore:** `close` writes at once, and reopening restores the source and position.
  - **Supersede:** a second `open` supersedes a running one.

### Task 8: Engine: metronome, count-in and loops
- routine: no
- files: `src/player.rs`, `src/player_tests.rs`
- does: 2.4 and 2.6 inside the engine and manager:
  - grid upload on a tempo change, and the metronome clicks;
  - the count-in rules (main Play, loop Play, repeats);
  - loop repeat with fades, the loop one-shot end, loop Play/Stop and `loop_playing`;
  - incomplete loops never repeat, and loop Play/flags on them are refused;
  - Prev/Next jumps;
  - `set_bpm` (tempo change, loop re-snap, grid upload, speed 1.0), and `set_metronome`;
  - the speed actions return "not available yet".
- done when: the tests pass with `ManualBackend` on "With Tempo" (22050 Hz):
  - **Metronome:** on from 0 ms, the clicks are at the frames for 0, 500, 1000 and 1500 ms (exact onset frames), with
    the accent at 500 and 2500, not at 0 (which is beat 4 of bar 0).
  - **Count-in:** Count In on + Play at 0 gives clicks at output frames 0, 11025 and 22050, silence until 33075, then
    the source from frame 0.
  - **loop-1 Play with Loop On:** after the loop length the output continues from 2500 ms (wrap counter 1,
    `player wrap` logged), with no count-in.
  - **loop-2 Play** (loop count-in on): a count-in at 90 BPM before the first pass and before every repeat.
  - **Loop On off:** the loop stops at its end, the position returns to its start, and `loop_playing` is false.
  - **Main Play** inside a repeating loop wraps at its end.
  - **Incomplete loop:** selecting loop-3 and pressing loop Play is refused with "Set the loop end first", and main
    Play from 18500 ms runs to the track end without a wrap.
  - **Tempo change:** `set_bpm(100)` at 3000 ms keeps the loops' times within half a tick, and loop-3 still has no end.
  - **Refusal:** the metronome is refused on a track without tempo.
  - **Real-time:** the callback never allocates (counting allocator in the test, as the Editor tests do).

### Task 9: IPC commands, shared backend, ticker, exit
- routine: no
- files:
  - Rust: `src/player_ipc.rs` (new), `src/gui.rs`, `src/ipc.rs` (`EditorState::new` takes the shared backend; Editor
    Play pauses the Player), `src/editor.rs` (`EditorManager::new` takes `Arc<dyn OutputBackend>`);
  - permissions: `build.rs`, `capabilities/main.json`;
  - frontend: `src/ui/lib/ipc.ts`, `src/ui/lib/ipc-player.test.ts` (new), `src/ui/lib/fixture-tracks.ts` (player
    fixture records)
- does:
  - the commands of 2.9 as thin async wrappers;
  - `PlayerHost` (the wrapper around `PlayerManager`, like `EditorState`), managed with the tools' ffmpeg path, and
    the ticker calls it;
  - `RunEvent::Exit` pauses, closes and flushes `player.json` (position included; 2 s cap);
  - Player Play stops the Editor;
  - TS types and wrappers (`openPlayer(id, onEvent)`, ...).
- done when:
  - the static command-list test in `tests/frontend.rs` passes;
  - `ipc-player.test.ts` checks each wrapper's command and argument names with `mockIPC`;
  - the existing Editor tests pass, with unchanged behaviour;
  - `npm test` passes.

### Task 10: Bar and BPM formatting
- routine: yes
- files: `src/ui/lib/bar-format.ts` (new), `src/ui/lib/bar-format.test.ts` (new), `src/ui/lib/bpm-format.ts` (new),
  `src/ui/lib/bpm-format.test.ts` (new)
- does:
  - `formatBar(tick: number, beatsPerBar: number): string`:
    - `perBar = 16 * beatsPerBar`;
    - `bar = Math.floor(tick / perBar) + 1`;
    - `rest = tick - (bar - 1) * perBar`;
    - `beat = Math.floor(rest / 16) + 1`;
    - `sub = rest % 16`;
    - returns `${bar}.${beat}.${sub}`.
  - `parseBar(text: string, beatsPerBar: number): number | null`:
    - trim, then match `^(-?\d+)\.(\d+)(?:\.(\d+))?$`;
    - beat must be 1..beatsPerBar and sub 0..15 (a missing sub = 0), else null;
    - returns `(bar - 1) * 16 * beatsPerBar + (beat - 1) * 16 + sub`.
  - `formatLoopTime(ms: number): string`:
    - `r = Math.round(Math.max(ms, 0))`;
    - `m = Math.floor(r / 60000)`, `s = Math.floor(r / 1000) % 60`, `f = r % 1000`;
    - returns `${m}:${pad2(s)}.${pad3(f)}` (zero-padded to 2 and 3 digits).
  - `parseLoopTime(text: string): number | null`:
    - trim, then match `^(\d+):([0-5]\d)(?:\.(\d{1,3}))?$` and return `(m*60+s)*1000 + frac`, where the fraction digits
      are right-padded with zeros to 3 digits;
    - else fall back to `parsePosition` from `./time-format`;
    - else null.
  - `formatBpm(bpm: number): string`: `0` -> `0`; integers without decimals; otherwise one decimal (`92.5`).
  - `parseBpm(text: string): number | null`: trim; `^\d+(?:\.\d)?$` else null; the value must be 0, or 20..300
    inclusive, else null.
- done when: `npx vitest run src/ui/lib/bar-format.test.ts src/ui/lib/bpm-format.test.ts` passes with at least these
  cases, and `npm run check` passes:
  - `formatBar`: (0,4) `1.1.0`; (64,4) `2.1.0`; (72,4) `2.1.8`; (16,4) `1.2.0`; (-16,4) `0.4.0`; (48,3) `2.1.0`;
  - `parseBar`: (`2.1.8`,4) 72; (`1.1`,4) 0; (`0.4.0`,4) -16; null for `1.5.0` (beats 4), `1.1.16`, `a.b`, ``;
  - `formatLoopTime`: 0 `0:00.000`; 67250 `1:07.250`; 13166.667 `0:13.167`; -3 `0:00.000`;
  - `parseLoopTime`: `1:07.250` 67250; `1:07.25` 67250; `1:07` 67000; `75.5` 75500; null for `1:7`, `x`;
  - `formatBpm`: 0 `0`; 120 `120`; 92.5 `92.5`;
  - `parseBpm`: `120` 120; `92.5` 92.5; `0` 0; null for `19.9`, `300.1`, `12a`, `92.55`, ``.

### Task 11: Waveform geometry
- routine: yes
- files: `src/ui/lib/waveform-geometry.ts` (new), `src/ui/lib/waveform-geometry.test.ts` (new)
- does:
  - `columns(peaks: number[], width: number): number[]`:
    - for `width <= 0` or empty peaks, return `[]`;
    - for each column `x` in `0..width-1`: `a = Math.floor(x * peaks.length / width)` and
      `b = Math.max(a + 1, Math.floor((x + 1) * peaks.length / width))`;
    - the value is the max of `peaks[a..b)` (b capped at `peaks.length`).
  - `xOf(ms: number, durationMs: number, width: number): number`: `durationMs <= 0` -> 0; else
    `Math.round(Math.min(Math.max(ms, 0), durationMs) / durationMs * (width - 1))`.
  - `loopMarks(loops: {start_ms: number; end_ms: number | null}[], durationMs: number, width: number): {x1: number; x2: number | null}[]`:
    `x1 = xOf(start_ms)`, `x2 = end_ms === null ? null : xOf(end_ms)`, in the same order.
- done when: `npx vitest run src/ui/lib/waveform-geometry.test.ts` passes with at least these cases, and
  `npm run check` passes:
  - `columns([0.1,0.5,0.2,0.9], 2)` = `[0.5, 0.9]`; `columns([0.1,0.5], 4)` = `[0.1, 0.1, 0.5, 0.5]`;
    `columns([], 10)` = `[]`;
  - `xOf(5000, 20000, 801)` = 200; `xOf(-1, 20000, 801)` = 0; `xOf(30000, 20000, 801)` = 800;
  - `loopMarks([{start_ms: 2500, end_ms: 6500}, {start_ms: 18500, end_ms: null}], 20000, 801)` =
    `[{x1: 100, x2: 260}, {x1: 740, x2: null}]`.

### Task 12: Player view layout, collapsible tree, select component, settings placeholders
- routine: no
- files:
  - views: `src/ui/views/PlayerView.svelte`, `src/ui/views/PlayerView.test.ts` (new);
  - player components: `src/ui/components/player/PlayerPane.svelte` (new, layout with placeholders for the sections),
    `src/ui/components/player/SourceSelect.svelte` (new);
  - shared components: `src/ui/lib/components/ui/select/**` (new), `src/ui/components/library/TrackBrowser.svelte`
    (optional `collapsed` rail mode);
  - registry and tests: `src/ui/lib/views.ts` (Player summary; Settings feature list and the two settings
    placeholders name `gui-audio-midi-settings`), `src/ui/lib/views.test.ts`, `src/ui/acceptance_frontend.test.ts`
    and `src/ui/App.test.ts` where they expect the Player placeholder or the settings feature names;
  - docs: `docs/licences.md` (the select component in the copied-components row)
- does:
  - the 2.8 layout and the Ctrl+L rail;
  - the Player rescans on mount like the Editor;
  - the dropdown component (large, keyboard-operable, both themes);
  - the tablature placeholder;
  - the `player tree collapsed=` log line;
  - the placeholder change of 2.8.
- done when:
  - vitest: the Player renders the shared tree, and selecting a track there sets `lib.selectedId`;
  - vitest: Ctrl+L hides the tree and shows the rail with the "Library" button; Ctrl+L or the button restores it; and
    `#view-heading` exists in both states;
  - vitest: the select lists the given options and reports a change;
  - vitest: the Settings view shows "This section will be filled by: gui-audio-midi-settings." for both placeholders;
  - the static CSP/style tests pass;
  - `npm test` passes.

### Task 13: Player frontend state
- routine: no
- files: `src/ui/lib/player-state.svelte.ts` (new), `src/ui/lib/player-state.test.ts` (new), `src/ui/App.svelte`
  (`leavePlayer()` when the view changes away from `player`)
- does:
  - the 2.10 state and actions;
  - it follows the selection like the Editor (open/close, stale snapshots ignored, `superseded` ignored);
  - coalesced seeks, events applied, the frontend log lines of 2.9, re-attach after a reload.
- done when: vitest with `mockIPC` passes:
  - no selection gives `inactive` and no `open_player`;
  - a playable track calls `open_player`, gives `active`, and logs `player active ... source=backing:backing sources=4`;
  - switching tracks closes and opens, and ignores a late snapshot;
  - `selectSource` calls `player_select_source`, logs, and afterwards the play button state is "Play";
  - a `transport` event updates the position and `bpm`;
  - leaving the view while playing calls `player_leave` and logs `player leave pause`;
  - 20 rapid seeks keep at most 2 calls in flight, with the last value last;
  - `npm test` passes.

### Task 14: Control bar sections (BPM, Metronome, Playback, Loops)
- routine: no
- files:
  - new: `src/ui/components/player/{BpmSection,MetronomeSection,PlaybackSection,LoopsSection}.svelte`,
    `src/ui/components/player/ControlBar.test.ts`;
  - TimeField props: `src/ui/components/editor/TimeField.svelte`, `src/ui/components/editor/TimeField.test.ts`,
    `src/ui/components/editor/MixLane.svelte` (uses the props);
  - `src/ui/components/player/PlayerPane.svelte`
- does: the 2.8 sections:
  - enables and labels, `aria-pressed` push buttons, BPM field editing;
  - the bar boxes in both modes, including the incomplete state;
  - the read-only loop slider and the loop status line (`loop_message`);
  - disabled `-`/`+`/`Reset`/`Detect` with the hint, and the "V" indicator.
- done when: vitest with `mockIPC` and the "With Tempo" / "Practice Song" records passes:
  - **BPM:** the field shows `120`; the V indicator is checked for "With Tempo" and not for a single segment; typing
    `90` + Enter calls `player_tempo` with `set-bpm` 90; `19` marks the field invalid with no call.
  - **Disabled buttons:** the metronome buttons and `-`/`+`/`Reset` are disabled in timestamp mode.
  - **Section box and bars:** the section box shows `1/3`; `bar1` shows `2.1.0` and `bar2` `4.1.0` for loop-1, and the
    `0:02.500` style in timestamp mode; typing `3.1.0` in `bar1` sends `set` with `{tick: 128}`.
  - **loop-3 current:** `bar2` is empty with the placeholder; Play/Stop, Loop On and Count-In are disabled; the loop
    slider is empty.
  - **Loop buttons:** `>`, `<`, `+`, `-`, Prev, Next, Play/Stop, Loop On and Count-In send the right `player_loop`
    actions; a snapshot with `loop_message` shows it in the status line.
  - **Loop slider:** it is not focusable and shows position 4500 ms in loop-1 as 50 %.
  - **Playback:** Play reads Pause while `counting_in`; Rew/FF send `player_skip` -5000/5000.
  - **Regressions:** the Editor's TimeField tests still pass.
  - `npm test` passes.

### Task 15: Waveform view
- routine: no
- files: `src/ui/components/player/WaveformView.svelte` (new), `src/ui/components/player/WaveformView.test.ts` (new),
  `src/ui/components/player/PlayerPane.svelte`
- does: 2.10:
  - canvas drawing with `waveform-geometry`, aware of the device pixel ratio;
  - a `ResizeObserver` redraw, and theme colours read from CSS variables;
  - loop marks (the current loop brighter; incomplete loops get the start line only) and the playhead;
  - `role="img"` with an `aria-label` that names the loop count.
- done when:
  - vitest (canvas context stubbed): drawing is requested with `columns(peaks, width)` for the measured width, and a
    resize redraws with the new width;
  - loop marks are drawn at the `loopMarks(...)` positions (two lines and a bar for loop-1, one line for loop-3);
  - the label says "Waveform, 3 loops";
  - `npm test` passes.

  (Pixels are checked in task 17.)

### Task 16: Player keyboard shortcuts
- routine: no
- files: `src/ui/views/PlayerView.svelte`, `src/ui/views/PlayerView.test.ts`
- does: the 2.8 keys with the focus rules.
- done when: vitest passes:
  - Space toggles play only outside text fields, sliders, buttons, the select and the tree;
  - `b`/`e` send `mark` start/end;
  - arrows send `select` -1/+1 outside the tree, and move in the tree when it has focus;
  - `L` toggles `repeat` (and does nothing on an incomplete loop);
  - `[`/`]` do nothing in increment A;
  - Ctrl+L works with the focus anywhere in the view;
  - `npm test` passes.

### Task 17: GUI end-to-end tests, visual review, docs
- routine: no
- files:
  - tests: `tests/gui_player_e2e.rs` (new), `tests/common/mod.rs` (a `start_player` helper copying `library-player`
    with `CALLIOPE_E2E_AUDIO=capture:<target/...wav>`; WAV reading and band-energy onset helpers moved here from
    `tests/gui_editor_e2e.rs`), `package.json` (`test:gui` adds `--test gui_player_e2e`);
  - docs: `docs/ui.md` ("Decided by the team" entries for the Player), `src/calliope-gui/README.md` (Player usage and
    keys), `docs/architecture.md` (only deviations)
- does: tests on display `:1`, with the window floated at 1280x800, keyboard first (data-testid bounds or verified
  coordinates for the mouse). They assert on log lines, `player.json` on disk and the capture WAV:
  1. Alt+5; select "Stems Only" -> `player active ... source=stem:bass position=0`. Ctrl+L ->
     `player tree collapsed=true` (screenshot); Ctrl+L -> false.
  2. "Practice Song" -> `source=backing:backing sources=4`.
  3. Play 2 s -> the capture has 330 Hz. While playing, choose "Guitar" in the dropdown -> `player source
     key=stem:guitar` and Rust `playing=false`. Within 1 s `player.json` has `"source": "stem:guitar"` and a
     `position_ms` of about 2000. Play again -> 660 Hz, not 330 Hz. Pause -> the position is stable.
  4. Time field `0:01.0`, BPM `120` + Enter -> `player bpm value=120 position=1000`, and the file has one segment at
     1000.
  5. Metronome On, Play 3 s -> click onsets in the capture (1.5/2 kHz band) every 500 ms +/- 5 ms. Then Off.
  6. Count In On, Play from a pause -> three clicks 500 ms apart, then the guitar tone starts 1500 ms after the first
     click (+/- 10 ms), with no click there.
  7. **Loops:**
     - `+` -> `player loop add ... complete=false`; the loop Play button is disabled (screenshot shows a start mark
       only);
     - play on and press `<` -> `complete=true`;
     - Loop On, loop Play for 2 passes -> `player wrap loop=1` twice;
     - Loop On off, loop Play -> it stops at the end (`playing=false`);
     - loop Count-In on -> `player wrap ... count_in=true`;
     - `>` after the end -> `complete=false`.
  8. Pause at a known position, close gracefully, restart with the same dirs, select "Practice Song" ->
     `source=stem:guitar mode=bpm loops=1 position=<the paused ms>`.
  9. "MP3 Backing" -> plays (`player open ... source=audio rate=44100`). "Missing Backing" -> `sources=1
     source=stem:vocals`. "Missing Audio" -> `player inactive id=...0306` and the message.
  10. Alt+1 while playing -> `player leave pause` and Rust `playing=false`.

  Every test asserts no `csp-violation` and that the backend is never `cpal`.

  **Visual review:** screenshots at 1280x800 and 1024x640 (tree shown and collapsed), in dark and light, of:
  - the active Player with complete and incomplete loops on the waveform;
  - the inactive state;
  - a count-in.

  Check readability at 1-2 m, control sizes, focus rings, the amber marks, and that nothing is cut off. Fix what is
  found and record the UI decisions.
- done when:
  - `DISPLAY=:1 npm run test:gui` passes three times in a row (all suites);
  - the reviewed screenshots are in `target/gui-shots/player-*.png` and listed in the task log;
  - `npm test` passes.

### Project licence (owner decision; independent of A, build it with A)

### Task 18: Switch the project licence from Apache-2.0 to MPL-2.0
- routine: no
- files:
  - licence text and templates: `LICENSE`, `src/calliope-gui/licence-templates/Apache-2.0.txt` (new, = the current
    `LICENSE` text), `src/calliope-gui/licence-templates/MPL-2.0.txt` (only if it differs from the official text);
  - licence fields: `Cargo.toml` (root, `package.license`), `src/calliope-gui/package.json`,
    `src/calliope-gui/package-lock.json` (the root package entries only);
  - notices: `src/calliope-gui/third-party-notices.ts`, `src/calliope-stems/THIRD-PARTY-NOTICES.txt` (regenerated);
  - docs: `README.md`, `docs/licences.md`;
  - test: `src/calliope-gui/tests/frontend.rs` (new static test)
- does: 2.14:
  1. Move the Apache-2.0 text from `LICENSE` to `licence-templates/Apache-2.0.txt`, and make `standardText()` read
     every licence, Apache included, from `licence-templates/`.
  2. Write the official MPL-2.0 text (from `https://www.mozilla.org/media/MPL/2.0/index.txt`) to `LICENSE`, and check
     that `licence-templates/MPL-2.0.txt` is identical to it.
  3. Set `MPL-2.0` in the root `Cargo.toml` (`package.license`, inherited by all three crates), in `package.json`, and
     in `package-lock.json` for the root package entries only (leave the dependencies' entries alone).
  4. Change the notices header to "Calliope is licensed under the Mozilla Public License 2.0.", then run
     `npm run notices:stems`.
  5. README "Licence": "Calliope is licensed under the [Mozilla Public License 2.0](LICENSE) (SPDX: `MPL-2.0`).
     Copyright 2026 Valentin Rusu." plus the Exhibit A sentence ("This Source Code Form is subject to the terms of
     the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one
     at https://mozilla.org/MPL/2.0/."), keeping the link to `docs/licences.md`.
  6. In `docs/licences.md`:
     - "Calliope itself ... is licensed under MPL-2.0";
     - reword the transitive-MPL paragraph (same licence as Calliope now; still used unmodified);
     - the `symphonia` sentence becomes "`symphonia` (MPL-2.0) was rejected by the owner because of its MPL-2.0
       licence, which is why `claxon` is used. With Calliope itself now under MPL-2.0, the owner may reconsider this;
       the rule stands until then.";
     - note how the Apache-2.0 standard text is now found in the notices section.
  7. Add a static test `project_licence_is_mpl_2_0`:
     - the root `Cargo.toml` has `package.license = "MPL-2.0"`;
     - `package.json` has `"license": "MPL-2.0"`;
     - `LICENSE` starts with `Mozilla Public License Version 2.0` and equals `licence-templates/MPL-2.0.txt` byte for
       byte;
     - `licence-templates/Apache-2.0.txt` starts with `Apache License`.
- done when:
  - `cargo metadata --format-version 1 --no-deps` reports `MPL-2.0` for `calliope-gui`, `calliope-lib` and
    `calliope-stems`;
  - `sha256sum LICENSE` equals the hash of the official text (downloaded to the scratch directory);
  - `grep -rn "Apache License 2.0\|Apache-2.0" README.md` finds nothing about Calliope's own licence;
  - the new static test passes, as do the notices tests (`dist_ships_third_party_notices_for_every_linked_crate`,
    `src/calliope-stems/tests/notices.rs`, the stems smoke test);
  - `npm test` passes.

### Increment B (after the owner approves the library, section 8)

### Task 19: Time-stretch dependency and licence record
- routine: no
- files: `src/calliope-gui/Cargo.toml`, `Cargo.lock`, `docs/licences.md`, `src/calliope-gui/README.md` (build
  prerequisites), `docs/architecture.md` (Build and run prerequisites), `src/calliope-gui/third-party-notices.ts`
  only if the bundled C++ licence needs a template
- does:
  - add the approved crate;
  - record its licence, and the licence of the C++ library it compiles in, from the package metadata and the vendored
    LICENSE files;
  - add the build prerequisites (signalsmith-stretch: `clang` for bindgen; Bungee: `cmake`; SoundTouch: none beyond
    a C++ compiler).
- done when:
  - `cargo build -p calliope-gui` succeeds;
  - the licence-record test in `tests/frontend.rs` passes;
  - the third-party-notices test passes, and the generated notices contain the C++ library's licence text;
  - `npm test` passes.

### Task 20: Stretcher (`stretch.rs`)
- routine: no
- files: `src/stretch.rs` (new), `src/main.rs`
- does: the 2.12 trait and its implementation; allocation only in `new`.
- done when: the module tests pass:
  - a 440 Hz stereo tone at speed 0.8 and at 1.25 keeps its dominant frequency within 1 % (Goertzel scan);
  - 10 s of input at speed 0.8 gives 12.5 s of output (+/- latency);
  - after `prime`, the first output frames are not silent;
  - no allocation in `process` (counting allocator).

### Task 21: Engine and state: speed
- routine: no
- files: `src/player.rs`, `src/player_tests.rs`, `src/player_state.rs`
- does: 2.12:
  - in the engine: the feed ratio, the position latency, priming, clicks at `/speed`, the count-in at `bpm * speed`;
  - the `speed-step`/`speed-reset` actions, refused at BPM 0;
  - typing a BPM resets the speed;
  - the speed is persisted.
- done when: the tests pass:
  - speed 1.0 is still bit-exact;
  - at 115/120 the position advances 0.9583 s per output second (+/- 1 %);
  - the metronome clicks have 115 BPM spacing in output time;
  - `-` at 120 gives `bpm 115`;
  - `Reset` gives 1.0 and the correct BPM at the position (also in the second tempo segment);
  - at BPM 0, `speed-step` is refused and the speed stays 1.0;
  - the limits are 0.50/1.50;
  - a loop repeats correctly at speed 0.8.

### Task 22: Frontend and GUI for speed
- routine: no
- files: `src/ui/components/player/BpmSection.svelte`, `src/ui/components/player/ControlBar.test.ts`,
  `src/ui/views/PlayerView.svelte` (`[`/`]`), `tests/gui_player_e2e.rs`, `docs/ui.md`
- does:
  - enable `-`/`+`/`Reset` in BPM mode only, and the keys;
  - an e2e step: at BPM 120, `-` x4 makes the transport show 100, and the capture keeps 660 Hz (+/- 1 %) while the
    position advances at 100/120.
- done when:
  - vitest covers the enables (disabled at BPM 0) and the calls;
  - `DISPLAY=:1 npm run test:gui` passes three times in a row;
  - `npm test` passes.

## 4. Test strategy

- **Whole headless suite:** `cd src/calliope-gui && npm test` (svelte-check, vitest, vite build,
  `cargo test --workspace`, clippy with `-D warnings`).
- **GUI suite:** `cd src/calliope-gui && DISPLAY=:1 npm run test:gui` (built with `e2e-hooks`).
- **Audio hardware stand-ins.** No test ever opens the real device or makes a sound.
  - `ManualBackend` in unit tests: the test pulls frames from the real render callback and compares them sample by
    sample (source copy, fades, click onsets, count-in timing, loop repeats).
  - `Engine` is also tested directly (`apply`/`render`), without any backend.
  - `NullBackend` with `CALLIOPE_E2E_AUDIO=capture:<abs path>` in GUI tests. The test analyses the WAV: tone presence
    per frequency (330/440/550/660 Hz sources) and click onsets in the 1.5-2 kHz band.
  - `CpalBackend` is never constructed by a test (existing static test).
- **ffmpeg** (non-FLAC sources) is the real local tool on the generated MP3 fixture (no network).
- **Pure logic:**
  - tempo map math and snapping;
  - loop operations (including incomplete loops and the `<`/`>` rules) and re-snapping;
  - `player.json` lenient parsing (including `end: null` and `position_ms`) and the source choice;
  - peaks, click synthesis and the count-in plan;
  - debounce and periodic position saving with injected `Instant`s (no sleeps for timing rules).
- **Frontend:** vitest + `mockIPC` for state, components, formatting and keys. `transport` events are simulated
  through the channel callback, and the canvas is stubbed.
- **End-to-end:** the real binary on `:1`, with temp XDG dirs and a copied `library-player` under `target/`. The tests
  check log lines, `player.json` on disk (including after a restart) and the capture WAV, and save screenshots for
  review.
- **Licence switch:** static tests (licence fields, `LICENSE` = the official text, templates) and the existing
  third-party-notices tests.
- **Time-stretch (increment B):** frequency and duration checks on synthetic tones. Real listening quality is a manual
  check.
- **No edge-AI server** is involved in this plan (detection is a later spec).

## 5. Deployment

Single machine: everything runs in calliope-gui on the laptop (`npm run build:app`). Nothing changes on archserver.
Increment B adds the build prerequisite of the approved stretch library:
- signalsmith-stretch: `clang`;
- Bungee: `cmake`;
- both need a C++ compiler, which is already in `base-devel`.

## 6. Manual checks (owner, laptop with real audio)

1. **Playback:** run `npm run build:app` and start `target/release/calliope-gui`. Press Alt+5 and select a real stem
   track with a saved backing:
   - the dropdown shows "Backing";
   - the waveform shows the song's loud and quiet parts;
   - Play plays the backing on the default output, and the time and slider move smoothly.
2. **Source change:** while playing, switch the dropdown to Guitar. Playback stops; Play continues with the guitar from
   the same place.
3. **Tempo and metronome:** pause on the first downbeat, type the song's BPM and press Enter. Turn the metronome On and
   play: the ticks fall on the beat, with the accent on beat 1, for the whole song (or until a tempo change).
4. **Count-in:** turn Count In on, pause and press Play. You hear three ticks, then the music starts on the fourth
   beat, in time.
5. **Loops:**
   - Press `+` (or `b`) at the start of a chorus: only a start mark appears and the loop's Play is disabled.
   - Let it play and press `<` (or `e`) at the end: the amber bar appears.
   - Turn Loop On and press the loop's Play: it repeats without a click or a gap at the joint.
   - Turn the loop's Count-In on: a count-in plays before each pass.
6. **Navigation:** add more loops and use Prev/Next (or the arrows). The position jumps at once, and the marks on the
   waveform match what you hear.
7. **Restore:** pause somewhere in the middle, close the app and start it again. The track, source, BPM, loops and the
   paused position are back. Look at `player.json` in the track folder: it is readable.
8. **MP3:** select an old plain backing track whose audio is MP3. It plays (needs ffmpeg).
9. **Ctrl+L** collapses the tree and gives the waveform more width; Ctrl+L brings it back.
10. **Leaving the view:** switch to the Library while playing. Playback pauses; back in the Player, Play resumes from
    the same place.
11. **Readability:** from 1-2 m in a dim room, the BPM, time, section number and loop marks are readable, and the
    important actions work from the keyboard (Space, b, e, arrows, L).
12. **Licence:** `LICENSE` and the About/notices mention MPL-2.0 (after task 18).
13. **Speed (increment B):** press `-` four times at 120 BPM. The song plays at 100 BPM with the same pitch and
    acceptable sound quality, and `Reset` returns to normal speed. At BPM 0, `-`/`+` are disabled.

## 7. Acceptance criteria and mapping

Accepted by the owner on 2026-10-11, with the owner's edit to criterion 8. Criteria 22, 24, 25, 26, 27 and 28 are
updated for the owner's answers on loops without an end (Q10) and the saved position (Q13). Criteria 30-34 are added
for those answers. Criterion 18 moves to the later detection spec.

| # | Criterion (Given / When / Then) | Req. | Tasks | Tests |
|---|---|---|---|---|
| 1 | Given the Player view, when the user presses Ctrl+L, then the tree collapses to a narrow rail; Ctrl+L again restores it and the selection is unchanged | 1 | 12, 16, 17 | vitest PlayerView; e2e step 1 |
| 2 | Given a track selected in the Library, when the user opens the Player, then the same track is selected and loaded | 1, 2 | 12, 13 | vitest; e2e |
| 3 | Given no track or a track without audio is selected, then the player pane shows an inactive message and its controls are disabled | 2 | 12, 13, 14 | vitest; e2e step 9 ("Missing Audio") |
| 4 | Given a track with audio is selected, then the pane shows the dropdown, the tablature area, the waveform and the control bar with BPM, Metronome, Playback and Loops | 2, 5-8b | 12, 14, 15 | vitest; screenshots |
| 5 | Given a stem track with a backing variant opened for the first time, then the dropdown lists the backings, the original mix (if kept) and the stems, and selects the backing | 4 | 4, 5, 7 | Rust sources/choose_source; e2e step 2 |
| 6 | Given a stem track without a backing opened for the first time, then the first stem is selected | 4 | 4, 5 | Rust; e2e step 1 |
| 7 | Given the user changes the dropdown, then `player.json` next to `track.json` records it within 1 s, and the choice is restored when the track is selected again or the app restarts | 3, 4 | 4, 7, 9 | Rust debounce; e2e steps 3, 8 |
| 8 | Given playback is running, when the user changes the dropdown, then the playback stops | 4, 13 | 7, 13 | Rust: `select_source` while playing gives `playing: false`, output closed, position kept; vitest; e2e step 3 (Rust `playing=false`) |
| 9 | Given the player pane, then the tablature area is an empty placeholder taking the remaining height | 6 | 12 | vitest; screenshots |
| 10 | Given a selected track, when the user presses Play, then audio plays, Play reads Pause, and the slider and the `M:SS.T` field follow the position; Pause holds the position | 11 | 7, 13, 14 | Rust; vitest; e2e step 3 |
| 11 | Given the time field, when the user types a time within the track, then playback jumps there; outside, the field is marked invalid | 11 | 14 | vitest TimeField |
| 12 | Given any position, when the user presses Rew or FF, then the position moves 5 s back or forward at once, clamped to the track | 11 | 7, 14 | Rust skip; vitest |
| 13 | Given a track without tempo, then the BPM field shows 0, the V marker is unchecked, the loop boxes show timestamps, and Metronome, Count In, `-`, `+` and `Reset` are disabled | 14, 15 | 3, 14 | Rust; vitest |
| 14 | Given a track without tempo, when the user types 120 at position p, then the whole track has 120 BPM with bar 1 at p, and it is saved | 14 | 3, 8, 14 | Rust set_bpm_at; e2e step 4 |
| 15 | Given a 120 BPM track, when the user types 90 at a later position, then 90 applies from that position (snapped to the nearest 1/64) to the end or the next change, the V marker is checked, and the BPM field shows 120 before and 90 after the change while playing | 14 | 3, 8, 14 | Rust; vitest; fixture "With Tempo" |
| 16 | Given tempo changes, when the user types 0 at a position, then the tempo in effect there is removed; removing the last one returns to timestamp mode with the loops kept at their times | 14 | 3, 4 | Rust |
| 17 | (Increment B) Given 120 BPM, when the user presses `-`, then the BPM field shows 115 and playback runs at 115/120 of the speed with the pitch unchanged; `+` goes up by 5; Reset returns to the correct BPM at the position; at BPM 0 the three buttons are disabled | 14 | 19-22 | Rust stretch/engine; vitest; e2e |
| 18 | (Moved to the later spec `gui-play-backing-track-detect`) Detect replaces the tempo map with the detected one | 14 | - | - |
| 19 | Given BPM mode and Metronome On, when playing, then a tick sounds on every beat of the tempo map, accented on beat 1; Off stops the ticks | 15 | 6, 8 | Rust onsets; e2e step 5 |
| 20 | Given BPM mode and Count In on, when the user presses Play, then three ticks sound one beat apart and playback starts on the fourth beat without a tick | 15 | 6, 8 | Rust exact frames; e2e step 6 |
| 21 | Given a loop with Loop On, when it is played with Count In on, then the count-in plays before the first pass only; with the loop's Count-In on, before every pass | 15 | 8 | Rust; e2e step 7 |
| 22 | Given the Loops section, when the user presses `+`, then a new loop starting at the position, without an end, becomes current, and the section box shows its number and the count; `-` removes the current loop | 16 | 4, 8, 14 | Rust; vitest; e2e step 7 |
| 23 | Given several loops, when the user presses Prev or Next, then the previous/next loop becomes current and the position jumps to its start at once | 16 | 4, 8, 14 | Rust; vitest |
| 24 | Given BPM mode, when the user presses `>` or `<`, then the loop start/end is set to the nearest 1/64 position (stored as a division, shown as bar.beat.tick); in timestamp mode the exact time is stored and shown as `M:SS.mmm` | 16 | 3, 4, 10, 14 | Rust; vitest |
| 25 | Given a current loop with an end, when the user presses its Play, then the position jumps to the loop start and the section plays; with Loop On it repeats, otherwise it stops at the end; the button reads Stop while playing and Stop stops (no pause) | 16 | 8, 14 | Rust; e2e step 7 |
| 26 | Given a current loop with an end, then the small slider shows the position inside the loop and cannot be moved by the user | 16 | 14 | vitest |
| 27 | Given the window is resized, then the waveform fills the width and is redrawn; loop sections with an end appear as amber vertical marks joined by an amber bar | 17 | 11, 15, 17 | vitest; screenshots |
| 28 | Given any player setting is changed (source, BPM, metronome, count-in, loops and their flags, current loop, playback position), then it is on disk within 1 s without a Save button and restored after reselecting the track or restarting | 3, 13 | 4, 7, 8 | Rust; e2e step 8 |
| 29 | Given playback is running, when the user leaves the Player view, then playback pauses at its position | (owner) | 7, 13 | vitest; e2e step 10 |
| 30 | Given a loop without an end is current, then its Play, Loop On and Count-In buttons are disabled, the `bar2` box and the in-loop slider are empty, the waveform shows only its start mark, and playback never repeats it | 16, 17 | 4, 8, 11, 14, 15 | Rust (loop-3); vitest; e2e step 7 |
| 31 | Given a loop without an end, when the user presses `<` at a later position, then the loop gets that end and becomes complete; at or before the start, nothing changes and "The end must be after the start" is shown | 16 | 4, 14 | Rust; vitest; e2e step 7 |
| 32 | Given a complete loop, when the user presses `>` at or after its end, then the start moves there and the end is cleared (the loop waits for a new `<`) | 16 | 4 | Rust; e2e step 7 |
| 33 | Given a loop without an end, when the track is selected again or the app restarts, then the loop is still there without an end | 3, 13 | 4 | Rust round trip ("With Tempo" loop-3); e2e step 8 |
| 34 | Given a track was paused at a position, when it is selected again or the app restarts, then the position is restored | 3 (owner) | 4, 7, 9 | Rust; e2e step 8 |

## 8. Open question

**Time-stretch library for increment B** (needs the owner's approval). Calliope is now MPL-2.0 (task 18).
- **`signalsmith-stretch` 0.1.3: recommended.**
  - It is an MIT Rust binding of the MIT Signalsmith Stretch C++ library (the one Qt Multimedia ships).
  - It brings no licence obligations beyond the notice, and the best-known sound quality of the three.
  - Building needs `clang` (bindgen). The binding is young, with one maintainer.
- **Bungee (`bungee-rs`/`bungee-sys` 0.1.1, MPL-2.0).**
  - Same licence as Calliope now, so no friction (file-level copyleft; we would use it unmodified).
  - It is built for real-time streaming with seeks, which suits loop jumps.
  - Building needs CMake and C++17. The bindings are young (0.1.1, one maintainer), and it is less proven on music
    than Signalsmith. A close second.
- **SoundTouch (LGPL-2.1; `soundstretch` is only its command-line tool).**
  - Static linking into an MPL-2.0 binary is allowed, because MPL-2.0 lists the LGPL as a compatible secondary
    licence. But every distributed binary must then let users relink with a modified SoundTouch: either ship object
    files, or rely on Calliope's public source, plus the LGPL text and source offer.
  - The quality is noticeably lower (WSOLA).
  - `soundstretch` processes whole files offline, so it can't follow live `-`/`+` changes.
  - Not recommended.

All three sit behind our own `Stretcher` trait, so switching later is a one-file change. The speed is limited to
50-150 %.

Owner answers applied (no longer open):
1. Acceptance criteria accepted (with the owner's edit to criterion 8).
2. Increment A now; detection becomes a later spec.
3. Licence switched to MPL-2.0 (task 18).
4. "Beat This!" for detection, in the later spec.
5. One set of settings per track.
6. As recommended, but `-`/`+` (and `Reset`) are disabled at BPM 0.
7. `player.json` next to `track.json`.
8. 4/4 for now, with `beats_per_bar` stored.
9. The V marker is a read-only indicator.
10. `+` creates a loop without an end; the rest as recommended.
11. Leaving pauses; Ctrl+L rail.
12. Our own synthesised click in the same stream.
13. The position is saved per track; Rew/FF 5 s; the original mix is never auto-selected.
14. Audio output and MIDI settings go to a later feature (`gui-audio-midi-settings`).
