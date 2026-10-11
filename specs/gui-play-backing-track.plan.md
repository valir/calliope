# Plan: Play a backing track (Player view)

Spec: `specs/gui-play-backing-track.md`. Architecture: `docs/architecture.md`. UI guide: `docs/ui.md`.
Path conventions: as in `docs/architecture.md` ("Paths in this document"): `src/*.rs`, `src/ui/...`, `tests/...`,
`build.rs`, `capabilities/`, `package.json` are inside `src/calliope-gui/`.

Revision 1 (2026-10-11): first draft. The spec's "Acceptance criteria" section is still the template, so section 7
proposes criteria for the owner to confirm (open question Q1). The spec has two requirements numbered 8; this plan
calls the second one **8b** (the control bar sections).

## 1. Summary

The Player view (Alt+5) gets the shared Library tree on the left (Ctrl+L collapses it to a narrow rail) and, on the
right, a player for the selected track. At the top, a dropdown picks what to play (the backing mix or a single stem).
Below it are an empty tablature area, a full-width waveform with the loop sections marked, and a control bar with BPM,
Metronome, Playback and Loops sections. Playback, the metronome tick, the count-in and loop repeats run in Rust, in
one audio stream, so they stay sample-locked. Every setting (source, tempo map, speed, metronome, loops) is saved to a
`player.json` file next to the track's `track.json` a moment after each change, with no Save button.

The spec is large, so the plan is in three **increments** (open question Q2):
- **A (this build):** everything except the two items below.
- **B, speed change** (tempo change that keeps the pitch, for `-`, `+` and `Reset`): needs a time-stretch library,
  which is an owner decision (Q3).
- **C, BPM detection** (`Detect`, several tempos per track, on the edge-AI server): needs a beat-tracking model on
  archserver and a server update (Q4). Recommended as its own spec; sketched in 2.13.

Increment A ships `-`, `+`, `Reset` and `Detect` as visible but disabled buttons with a hint, so the layout is final.

## 2. Design

### 2.1 What is reused, what is new

Reused unchanged: `audio_out` (all three backends, the `e2e-hooks` rule that GUI tests can only use `NullBackend`),
`transport` (Play/Pause and the 0.3 s scrub-resume rule), `stem_audio::decode_i16` and `probe` (FLAC via `claxon`),
`fsutil::write_atomic`, the `TrackBrowser` tree column and the shared `lib` selection, the IPC/channel pattern, the
`Slider`/`Checkbox` components, `time-format.ts`.

New, all in the GUI crate:

| Module | Responsibility | Pure? |
|---|---|---|
| `src/tempo_map.rs` | Tempo segments, beat grid in 1/64-note ticks, ms<->tick, snapping, "set BPM at position", beats in a range | yes |
| `src/player_state.rs` | `player.json` schema (lenient read, strict write), loop list operations, re-snapping loops after a tempo change, the source-choice rule, load/save of the file | yes (file IO through `fsutil`) |
| `src/player_source.rs` | The dropdown entries of a track; decoding one entry into memory (FLAC: `claxon`; anything else: the external `ffmpeg` through a pipe); waveform peaks | yes except the ffmpeg child |
| `src/metronome.rs` | The click sound (synthesised, our own), count-in plan, placing ticks inside an output block | yes |
| `src/player.rs` (+ `src/player_tests.rs`) | `PlayerManager`/`PlayerSession`, the render `Engine` owned by the audio callback, commands, events, persistence debounce | engine is pure; manager like `editor.rs` |
| `src/player_ipc.rs` | The `#[tauri::command]` wrappers for the Player | Tauri glue |

`src/ipc.rs` is already 1400 lines, so the Player's commands go in their own `src/player_ipc.rs`. They follow every
rule of the IPC pattern (thin wrappers, registered in `gui.rs`, listed in `build.rs` and `capabilities/main.json`;
the static test already strips the `module::` prefix). This is recorded in the architecture.

### 2.2 The dropdown: sources of a track (spec req. 4)

`player_source::sources(track: &TrackRecord) -> Vec<Source { key, label, file }>`, in this order:

| Key | Label | When |
|---|---|---|
| `backing:<id>` | the variant's `name` (e.g. "Backing") | each entry of `backings` (stem tracks) |
| `audio` | "Backing track" | `audio` is set (plain backing tracks) |
| `original` | "Original mix" | `original` is set (kept at import) |
| `stem:<name>` | stem name with a capital first letter | each entry of `stems` |

Entries whose file is in the record's `missing` list are left out. A track with no entry is "not playable" (the pane
shows "<title> has no audio to play."). **Default** (first visit, or the saved key no longer exists): the first
`backing:*`, else `audio`, else the first `stem:*` (the spec's rule); `original` is never chosen automatically (Q13).
Changing the dropdown is saved at once (2.5) and logged.

**Decoding** (`player_source::decode(file, ffmpeg: Option<&Path>, cancel, progress) -> Pcm {rate, channels, frames,
samples: Vec<i16>}`):
- `.flac`: `stem_audio::decode_i16` (16-bit in memory, as in the Editor).
- Anything else (e.g. `backing.mp3` of a plain backing track): `ffmpeg -nostdin -hide_banner -loglevel error
  -protocol_whitelist file,pipe -i file:<abs> -map 0:a:0 -f s16le -acodec pcm_s16le -ac 2 -ar 44100 pipe:1`, run with
  `calliope_lib::process` (argv, own process group, PDEATHSIG, cancel), read straight into memory. No file is written
  and no MPL decoder is needed (`symphonia` stays rejected). Without ffmpeg: "Install ffmpeg to play <file>."
- Same 1.5 GiB cap as the Editor (one source is at most about 160 MB at 15 minutes).

**Peaks** for the waveform: `peaks(&Pcm, buckets = 2048) -> Vec<f32>`, the maximum `|sample|/32768` over all channels
in each of 2048 equal slices of the track (the last slice takes the remainder). Computed once per decode, sent in the
snapshot (2048 numbers), and drawn by the frontend at any width (2.10).

### 2.3 Tempo map and beat grid (spec req. 14)

```
tempo: { beats_per_bar: 4, segments: [ {start_ms: 500.0, bpm: 120.0}, {start_ms: 10500.0, bpm: 90.0} ] }
```

- **BPM 0 = no segments.** A new track has none: the BPM field shows `0`, loops are in **timestamp mode** and the
  metronome/count-in buttons are disabled. One or more segments = **BPM mode**.
- **Grid.** A beat is a quarter note (`beats_per_bar` = 4, stored so a later feature can change it; Q8). The grid unit
  is a **tick = one 1/64 note = 1/16 beat**, so a 4/4 bar has 64 ticks. Segment *i* has a tick length
  `d_i = 60000 / (bpm_i * 16)` ms and a tick base `T_i` (`T_0 = 0`, `T_{i+1} = T_i + round((start_{i+1} - start_i) / d_i)`).
  The first segment's start is the **anchor**: tick 0 = bar 1, beat 1. Before the anchor, the first segment's tempo
  extends backwards (negative ticks), so one BPM typed at the first downbeat covers the whole track.
- `ms_at(tick)`, `tick_near(ms)` (nearest tick, ties away from zero), `bpm_at(ms)`, `segment_at(ms)`,
  `beats_in(ms_a..ms_b) -> iterator of (ms, accent)` (accent = first beat of a bar), `is_multi()` (= more than one
  segment; drives the **"V" marker**, a read-only check indicator, Q9).
- **Typing a BPM** (`set_bpm_at(position_ms, bpm)`; valid values: 0, or 20.0..=300.0 with one decimal):
  - no segments, bpm > 0: one segment starting **exactly at the position** (the user pauses on a downbeat and types
    the tempo; that anchors the grid);
  - otherwise the position is first **snapped to the nearest tick of the current grid**, so tick bases stay integers.
    If a segment starts there (or the position is before the anchor), that segment's bpm is replaced; else a new
    segment starts there. It lasts until the next change or the end (spec);
  - bpm 0 removes the segment in effect at the position (the last one removed = no tempo, timestamp mode);
  - neighbouring segments with the same bpm are merged (same grid, nothing audible changes).
- **Loops follow the audio, not the bar numbers, when the tempo map changes**: every loop boundary is turned into
  ms with the old map and back into the nearest tick with the new one (or kept as ms when the new map is empty;
  ms boundaries become ticks when the first tempo is entered). So a loop stays where the user set it, and its stored
  kind always matches the mode (spec: "timestamp mode saves absolute timestamps, BPM mode only saves the closest time
  division").
- **Speed** (increment B; 2.12): one factor for the whole track, `speed` (1.0 = original). The BPM field always shows
  `bpm_at(position) * speed`, so it follows tempo changes while playing. Increment A keeps `speed = 1.0`.

### 2.4 Loops (spec req. 12, 16)

```
loops: [ {id: "loop-1", start: {tick: 64}, end: {tick: 192}, repeat: true, count_in: false} ], current_loop: "loop-1"
```

- A boundary is `{tick: i64}` (BPM mode) or `{ms: f64}` (timestamp mode). Loops are kept **sorted by start**; the
  current loop is remembered by id, so its number can change when another loop is moved before it. At most 99 loops.
- `+` adds a loop starting at the position (snapped in BPM mode) and ending 4 bars later (BPM mode) or 10 s later
  (timestamp mode), clamped to the track; it becomes current. `-` removes the current loop (no confirmation; Q10);
  the next one (or the previous, if it was the last) becomes current.
- `Prev`/`Next` select the previous/next loop and **jump the position to its start at once** (no 0.3 s delay);
  playing continues from there.
- `>` sets the current loop's start to the position, `<` its end. BPM mode snaps to the nearest tick (forward or
  backward, whichever is closer). If start would end up after end, the two are swapped; equal values are refused
  (nothing changes).
- The `bar1`/`bar2` boxes show the start/end and are editable (Enter commits, Escape reverts, invalid input marks
  the field like the time field):
  - BPM mode: `bar.beat.tick`, e.g. `5.1.0`, `12.3.8` (bar from 1, beat 1..beats_per_bar, tick 0..15); bars before
    the anchor are 0 or negative;
  - timestamp mode: `M:SS.mmm`, e.g. `1:07.250`; input also accepts the time field's forms.
- The section box is read-only and shows `2/5` (current / count), or `-` when there are no loops.
- The small slider is read-only (`aria-readonly`, not focusable): the position inside the current loop, 0..100 %;
  empty when the position is outside the loop.
- `Loop On` (`repeat`, per loop): while it is on, **any playback that reaches the current loop's end while inside the
  loop jumps back to its start** (main Play too, the usual A-B repeat; Q10). `Count-In` (per loop) plays a count-in
  before every repeat (2.6).
- Loop `Play` jumps to the loop start and plays (with a count-in, 2.6). It reads `Stop` while that loop is playing.
  With `Loop On` off, it plays the section once and stops at the end; the position returns to the loop start.
  `Stop` stops and returns to the loop start. There is no loop pause (spec); the main Pause still pauses anything.
- Speed changes (increment B) apply to loop playback automatically (one stream).

### 2.5 Persistence: `player.json` next to `track.json` (spec req. 3, 4, 13)

```json
{
  "schema_version": 1,
  "source": "stem:guitar",
  "tempo": {"beats_per_bar": 4, "segments": [{"start_ms": 500.0, "bpm": 120.0}]},
  "speed": 1.0,
  "metronome": {"on": false, "count_in": true},
  "loops": [{"id": "loop-1", "start": {"tick": 64}, "end": {"tick": 192}, "repeat": true, "count_in": false}],
  "current_loop": "loop-1"
}
```

- **Where:** `<root>/tracks/<id>/player.json` (Q7). It is the "next to the track" file of req. 4. Reasons: the Player
  writes after every adjustment, and writing `track.json` would change its revision each time, so a Library edit in
  progress would hit a save conflict; older builds ignore an unknown file; it is plain JSON, easy to read and fix by
  hand. `player.json` becomes a **reserved file name** like `track.json` (`fsutil::validate_file_name`), and **Export**
  copies it when present. Delete moves the whole folder to the trash as before. Library scans ignore it.
- **One state per track, shared by all sources** (Q5): the stems and backings of a track are sample-aligned, so the
  tempo map, loops, speed and metronome apply to whichever source is selected. Only `source` records the choice.
- **When:** each change marks the session dirty; the ticker writes the file (`write_atomic`) 500 ms after the last
  change, and at once on track switch, `close_player`, view leave and app exit. "Persisted as soon as adjusted" is
  checked as "on disk within 1 s".
- **Never creates folders**: if the track folder is gone (deleted meanwhile) the write fails, the pane shows "Could
  not save the player settings: ..." inline, and the session keeps working.
- **Lenient read:** a missing file = defaults; an unreadable or invalid file = defaults plus an inline note "Player
  settings in player.json could not be read; they will be replaced when you change something". Bad parts are dropped
  individually (a bad loop is skipped, an unknown source key falls back to the default, bpm out of range drops that
  segment, `speed` is clamped). Unknown fields are kept on write. A newer `schema_version` = read-only: the Player
  plays with defaults and never overwrites the file (same rule as `track.json`).
- Not saved: the playback position (a track opens at `0:00.0`), playing state, tree collapse (Q11, Q13).

### 2.6 Metronome and count-in (spec req. 10, 15)

- **One stream.** Clicks are mixed by the same render callback that plays the track, after the (future) speed
  change, so a tick is sample-aligned with the music whatever the device latency (Q12). The output is hard-clipped at
  full scale like the Editor mix.
- **Sound** (`metronome::Click`): our own synthesis, no samples from anywhere: a 2 kHz sine (accent: first beat of a
  bar) or 1.5 kHz (other beats) with an exponential decay over 30 ms, peak 0.5 (-6 dBFS). Rendered into a
  pre-allocated buffer at the output rate when a session opens.
- **Metronome On** (requires BPM mode, otherwise the button is disabled): a click on every beat of the grid
  (`tempo_map::beats_in`) inside the played range, mapped to output frames.
- **Count-in** (`Count In`, metronome section; requires BPM mode): `count_in_plan(bpm_at(start) * speed, rate)` gives
  three audible clicks one beat apart, then playback starts on the fourth beat, which is silent (spec): clicks at
  output frames 0, B, 2B, audio from 3B, where B is one beat. During it the position does not move and the transport
  reports `counting_in: true`; Pause cancels it.
- **When a count-in plays** (Q10):
  - main Play from stopped or paused, if `Count In` is on (not on a resume after a scrub);
  - loop Play, if `Count In` is on **or** that loop's `Count-In` is on;
  - each repeat of a loop (jump back to its start), only if that loop's `Count-In` is on.
- Count-in and the metronome are independent: the count-in plays even with the metronome off.

### 2.7 The engine (Rust, real-time)

The Player is a second, independent engine next to the Editor's (`player.rs`), because it adds count-ins, loop
repeats and clicks that must happen at an exact sample without waiting for the 20 ms ticker. Its playback state
lives **inside the audio callback**:

- `Engine` (pure struct, unit-tested without any backend): the decoded source (`Arc<Pcm>`), position (frames),
  run state `Stopped | Paused | CountIn { left, next_click } | Playing`, the `audible` gate of the scrub rule,
  the current loop (`start`, `end` frames, `repeat`, `count_in_each`), a one-shot "stop at loop end", the beat grid
  (`Arc<Vec<(frame, accent)>>`), metronome on, click voice, and (increment B) the stretcher.
  `fn apply(&mut self, cmd: Cmd)` and `fn render(&mut self, out: &mut [f32])`. It never allocates or locks.
- The callback closure owns the `Engine` and a `std::sync::mpsc::Receiver<Cmd>` from a bounded `sync_channel(64)`
  (the array flavour: no allocation per message). It drains commands, then renders. Replaced `Arc`s (old source, old
  grid) go back through a second bounded channel so they are freed on the ticker thread, not in the callback.
- Status out of the callback is atomics: position, run state, `loop_playing`, a wrap counter and an "ended" flag.
- **Discontinuities** (seek, loop repeat, start after a count-in, source change) get a 3 ms linear fade-in, and a
  known jump point (loop end, one-shot stop) a 3 ms fade-out before it, so loops repeat without clicks. Everything
  else is bit-exact copies of the source (speed 1.0), which the tests compare sample by sample.
- `PlayerManager` (Tauri-managed behind a `Mutex`, same shape as `EditorManager`): the session (track id, dir,
  sources, state, tempo map, `Arc<Pcm>`, peaks, `Transport`, output handle + command sender), the deferred-drop trash
  for output handles, the dirty flag. Opening and source decoding run without the manager lock (ticket +
  generation, as in the Editor). The device is opened outside the lock (`OutputRequest` pattern). When no output is
  open the manager's own copy of position/state is authoritative; opening the output builds the `Engine` from it.
- **Source change while playing:** the new source is decoded off the lock, then swapped in by a `Cmd::SetSource`
  at the current position (fade-in), so playback continues where it was. A different sample rate (e.g. an mp3
  decoded at 44.1 kHz next to 22.05 kHz stems) closes and reopens the output at the new rate.
- **Ticker:** the existing 20 ms thread in `gui.rs` also calls `player_ipc::PlayerHost::tick(now)` (the Tauri-managed
  wrapper, like `EditorState`): applies a due scrub resume
  (`Cmd::Resume`), turns the callback's flags into state (loop one-shot end, end of track = stop at 0:00.0),
  reports device errors, writes a dirty `player.json`, and emits `transport` events (on change, else every 100 ms
  while playing or counting in).
- **One sound at a time:** Play in the Player stops the Editor (and Editor Play pauses the Player) through the IPC
  layer, so two streams never play at once. Views already stop/pause on leave; this is the safety net.
- Both managers share one `Arc<dyn OutputBackend>` built by `select_backend()` in `gui.rs`
  (`EditorManager::new` takes the `Arc`), so the GUI tests' capture file is one WAV.

### 2.8 Behaviour (spec gaps filled; owner decisions are open questions)

- **Player pane states:** no selection ("Select a track to play it."), not playable ("<title> has no audio to
  play."), loading ("Loading..." with progress), error (inline alert), active. Inactive states show the layout with
  disabled controls.
- **Selection** is the shared `lib.selectedId`: selecting a track in the Player selects it in Library and Editor,
  and the other way round. A Library edit in progress locks the tree (existing behaviour).
- **Track change while playing:** the session is closed (state flushed to disk) and the new track opens stopped at
  `0:00.0`.
- **Leaving the Player view:** `player_leave` = pause (position kept), the output is closed and the state flushed
  (Q11). Coming back shows the same track paused at the same position.
- **Playback section:** `Play/Pause` (reads `Pause` while playing or counting in), the position slider (0..duration,
  step 100 ms, 0.3 s scrub rule while playing), `Rew`/`FF` jump -5 s/+5 s at once, clamped (Q13), and the time field
  `M:SS.T` (the Editor's format and parser, wheel = -/+0.1 s, Enter commits, out-of-range refused with "Enter a time
  between 0:00.0 and <duration>"). End of track = stop at `0:00.0` (like the Editor).
- **BPM section:** the BPM field (label "BPM"; shows the BPM at the position, or `0`; while it is focused and edited,
  transport events don't overwrite it; Enter commits `set_bpm_at(position)`, Escape reverts; invalid input: "Enter a
  BPM between 20 and 300, or 0 to clear"), `-`, `+`, `Reset`, `Detect` (disabled in increment A with the hint "Comes
  in a later update"), and the "V" indicator (checked when the track has several tempos).
- **Metronome section:** `On/Off` and `Count In` push buttons (`aria-pressed`, amber when on), disabled without a
  tempo.
- **Loops section:** three groups, as in req. 12: (`Prev`, `+`, section box, `-`, `Next`), (`>`, `bar1`, small slider,
  `bar2`, `<`), (`Play/Stop`, `Loop On`, `Count-In`). Buttons that need a current loop are disabled without one.
- **Keyboard** (the UI guide's list, active in the Player when the focus is not in a text field, slider, button,
  select or the tree, like the Editor's Space rule): `Space` Play/Pause, `b` = `>`, `e` = `<`, `←`/`→` = Prev/Next,
  `L` = Loop On, `[`/`]` = `-`/`+` (increment B; the spec's 5 BPM step, not the guide's 5 %), `Ctrl+L` = collapse the
  tree (works everywhere in the Player view).
- **Ctrl+L collapse** (Q11): the tree column (20 rem) becomes a 3.5 rem rail with a single large "Library" icon button
  (tooltip "Show the library (Ctrl+L)") that expands it again; the `h1#view-heading` "Player" stays in the rail
  (visually hidden) for focus. Not remembered between runs (like `Ctrl+B`). Player view only.
- **Layout:** right pane = dropdown row (label "Play:", the select, the track title); the tablature area takes the
  remaining height (dashed placeholder "The tablature will be shown here (gui-tablatures)."); below it, pinned, the
  waveform lane (about 6 rem high, full width) and the control bar. The bar's four sections are bordered groups in a
  wrapping row: at 1280x800 they fit in two rows (BPM + Metronome + Playback, then Loops); at 1024x640 with the tree
  collapsed the same; nothing scrolls out of view. Control sizes follow the guide (buttons at least ~50 px tall).
  Final details are fixed in the visual review task.
- **Select component:** shadcn-svelte `select` (bits-ui `Select`), copied into `src/ui/lib/components/ui/select/` like
  the other components (MIT, recorded in `docs/licences.md`).

### 2.9 IPC (pattern of `docs/architecture.md`)

Rust types in `src/player.rs`, commands in `src/player_ipc.rs`, TS mirrors in `src/ui/lib/ipc.ts`. Every command
takes the track `id` and fails with "no player session for <id>" when it isn't the open one.

```ts
type Boundary = { tick: number } | { ms: number };
interface PlayerLoop { id: string; start: Boundary; end: Boundary; start_ms: number; end_ms: number;
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
  loops: PlayerLoop[]; current_loop: number | null /* index */;
  transport: PlayerTransport; can_change_speed: boolean; can_detect: boolean; note: string | null /* settings file note */
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

| Command | Returns | Notes |
|---|---|---|
| `open_player(id, events: Channel<PlayerEvent>)` | `PlayerSnapshot` | async + spawn_blocking; closes (and flushes) the previous session; a superseded open fails with `superseded` |
| `close_player()` | `()` | flushes `player.json` |
| `get_player()` / `watch_player(events)` | `PlayerSnapshot \| null` | re-attach after a webview reload |
| `player_select_source(id, source)` | `PlayerSnapshot` | decodes off the lock; keeps position and playing state |
| `player_play(id)`, `player_pause(id)` | `PlayerTransport` | Play applies the count-in rule |
| `player_seek(id, position)`, `player_nudge(id, delta)` | `PlayerTransport` | ms; clamped; 0.3 s scrub rule while playing |
| `player_skip(id, delta)` | `PlayerTransport` | Rew/FF; instant jump, clamped |
| `player_leave(id)` | `PlayerTransport` | pause, close the output, flush |
| `player_tempo(id, action: TempoAction)` | `PlayerSnapshot` | speed kinds fail with "Speed change is not available yet" in increment A |
| `player_metronome(id, on, count_in)` | `PlayerSnapshot` | refused without a tempo |
| `player_loop(id, action: LoopAction)` | `PlayerSnapshot` | |

Fourteen new commands (plus `player_detect`/`cancel_player_detect` in increment C), each in `generate_handler!`,
`COMMANDS` in `build.rs` and `capabilities/main.json`.

**Log lines** (for the GUI tests):
- Frontend (`calliope-ui: `): `player active id=<id> source=<key> sources=<n> mode=<bpm|time> loops=<n>`,
  `player inactive id=<id|none>`, `player source key=<key>`, `player play|pause position=<ms>`,
  `player seek position=<ms>`, `player skip delta=<ms>`, `player bpm value=<v> position=<ms>`,
  `player metronome on=<b> count_in=<b>`, `player loop <action> index=<n|none> count=<n>`,
  `player tree collapsed=<b>`, `player leave pause`, `error player <command>: <msg>`.
- Rust (`calliope: `): `player open id=<id> source=<key> duration_ms=<ms> rate=<hz>`,
  `player transport playing=<b> counting_in=<b> loop=<n|none> position_ms=<ms>` (changes only),
  `player count-in clicks=3 beat_ms=<ms>`, `player wrap loop=<n> count_in=<b>`, `player state saved id=<id>`,
  `player state error id=<id> message=<m>`.

### 2.10 Frontend modules

| File | Content |
|---|---|
| `src/ui/lib/bar-format.ts` | `formatBar(tick, beatsPerBar)`, `parseBar(text, beatsPerBar) -> number \| null`, `formatLoopTime(ms)` (`M:SS.mmm`), `parseLoopTime(text) -> number \| null`; pure |
| `src/ui/lib/bpm-format.ts` | `formatBpm(bpm)` (`0`, `120`, `92.5`), `parseBpm(text) -> number \| null` (0 or 20..300, one decimal); pure |
| `src/ui/lib/waveform-geometry.ts` | `columns(peaks, width) -> number[]` (max of the peaks under each pixel column), `xOf(ms, durationMs, width)`, `loopMarks(loops, durationMs, width)`; pure |
| `src/ui/lib/player-state.svelte.ts` | Module `$state` `pl` (status, snapshot fields, transport, messages, `treeCollapsed`) and the actions (`syncSelection`, `leavePlayer`, `selectSource`, `togglePlay`, `seek` (coalesced), `nudge`, `skip`, `setBpm`, `setMetronome`, `loop(action)`, `attachPlayer`), the event handler, stale-id guard, log lines |
| `src/ui/lib/components/ui/select/` | shadcn-svelte select over bits-ui, large size, both themes, CSP-safe |
| `src/ui/components/editor/TimeField.svelte` | generalised with props (`positionMs`, `durationMs`, `active`, `onseek`, `onnudge`) and used by both views |
| `src/ui/components/player/PlayerPane.svelte` | states, dropdown row, tablature placeholder, waveform lane, control bar |
| `src/ui/components/player/{SourceSelect,WaveformView,BpmSection,MetronomeSection,PlaybackSection,LoopsSection}.svelte` | the parts; `WaveformView` draws on a `<canvas>` (scripted drawing is CSP-safe), redraws on `ResizeObserver` and on changes; amber loop marks (2 px lines at start/end, a 4 px bar joining them along the top, the current loop brighter), a playhead line |
| `src/ui/views/PlayerView.svelte` | `TrackBrowser` (or the collapsed rail) + `PlayerPane`; keys |

### 2.11 Decisions inside the answers (stated for review)

- BPM input: 0 or 20..300 with one decimal; the field shows integers without decimals.
- New loop: 4 bars / 10 s long. Max 99 loops. `>`/`<` swap a reversed pair; equal is refused.
- Loop marks and the playhead are drawn by the frontend from ms values in the snapshot/events (no time kept in JS).
- Peaks: 2048 max-abs values per source.
- Fades: 3 ms at jumps.
- The `transport` event interval stays 100 ms (smooth enough for the waveform playhead; the tablature cursor in
  `gui-tablatures` may need more).

### 2.12 Increment B: speed change keeping the pitch (sketch; tasks 18-21)

- `src/stretch.rs`: `trait Stretcher { fn reset(); fn prime(&mut self, pre_roll: &[f32]); fn process(&mut self, input:
  &[f32], output: &mut [f32]); fn latency_frames() -> usize }`. Implementation per Q3 (recommended:
  `signalsmith-stretch`, which binds the MIT Signalsmith Stretch C++ library; built with `cc`, bindgen needs `clang`
  at build time). At speed 1.0 the engine bypasses it (bit-exact path of increment A).
- Engine: per output block of N frames it feeds `round(N * speed)` source frames (fractional remainder carried), so
  the position advances at `speed`; the reported position subtracts the stretcher latency (scaled by speed); seeks
  and loop repeats call `prime` with the 100 ms before the target. Clicks are placed at `(beat - pos) / speed`.
  Count-in uses `bpm * speed`.
- `-`/`+`: target = shown BPM -/+ 5, speed = target / `bpm_at(position)`, clamped to 0.50..1.50 (Q3). Without a
  tempo they step the speed by 5 % and a "Speed 95 %" readout appears next to the field (Q6). `Reset` = speed 1.0
  (the field shows the correct BPM at the position; with no tempo it stays `0`). Typing a BPM resets the speed to 1.0
  (Q6). Speed is saved in `player.json`.

### 2.13 Increment C: BPM detection on the edge-AI server (sketch; recommended as its own spec)

- **Server** (`calliope-stems`, additive to API v1, Q4): `POST /v1/beats` (FLAC body, same limits) -> `202 {"job"}`;
  `GET /v1/beats/<job>` -> `{"state", "progress", "beats": [s...], "downbeats": [s...]}`; `DELETE` as for stems. A new
  optional `--beat-tracker PATH` runs `<tracker> <input.flac> <out.json>` (adapter script, like the separator);
  `GET /v1/health` gains `"tasks": ["stems", "beats"]` (absent `beats` = Detect disabled with a message). Recommended
  model: "Beat This!" (CPJKU, MIT code and weights as listed; to be verified when the spec is written) in its own venv
  on archserver.
- **App** (pure, `calliope-lib` or `tempo_map`): beats -> tempo segments (inter-beat intervals, median per 8 beats, a
  new segment where the median moves more than 4 % for at least 8 beats), first downbeat = anchor, bar length from the
  downbeats; the result replaces the tempo map (loops re-snapped, 2.3) and resets the speed. Detect uploads the
  selected source as FLAC (ffmpeg for non-FLAC), as an import-style job with progress in the BPM section.
- **Tests:** a stub beat tracker (bash, canned beat lists incl. a two-tempo case) used with the real server on
  127.0.0.1, protocol conformance additions, segmentation unit tests. Deployment: the owner redeploys
  `calliope-stems` and installs the tracker venv (section 5).

## 3. Tasks

Every task ends with the headless suite green: `cd src/calliope-gui && npm test`. GUI tasks also run
`cd src/calliope-gui && DISPLAY=:1 npm run test:gui`. Tasks 1-17 are increment A. Tasks 18-21 (B) and 22-25 (C) are
only built after the owner's answers to Q2-Q4.

### Increment A

### Task 1: Player test fixtures
- routine: no
- files: `src/calliope-gui/tests/fixtures/player/make-fixtures.sh` (new), `src/calliope-gui/tests/fixtures/library-player/**`
  (generated, committed), `src/calliope-gui/tests/fixtures/import/README.md` (one line pointing to the script)
- does: a bash script in the style of `tests/fixtures/editor/make-fixtures.sh` (same `ff()` helper, bitexact flags,
  idempotent, `FORCE=1`, deterministic `track.json`). Repository `library-player/` with marker `{"schema_version": 1}`,
  album "Player Tests", 22050 Hz s16 FLAC, 20 s each unless stated:
  - `0199c0a0-0000-7000-8000-000000000301` "Practice Song": type stem; stems vocals (440 Hz mono), guitar (660 Hz
    mono); `backings/backing.flac` 330 Hz stereo with an amplitude envelope (0-5 s 0.2, 5-10 s 0.9, 10-15 s 0.5,
    15-20 s 0.9, via the lavfi `volume` filter with `eval=frame`), listed as variant `backing`/"Backing" (fields as in
    the Editor fixture "Mixed Before"); `original.flac` 550 Hz stereo, `"original": "original.flac"`.
  - `...0302` "Stems Only": type stem; stems bass (110 Hz mono), drums (220 Hz mono); no backings.
  - `...0303` "MP3 Backing": type backing; `audio` "backing.mp3" = 330 Hz stereo 44.1 kHz MP3 (`-c:a libmp3lame -b:a 128k`).
  - `...0304` "With Tempo": the same stems and backing as "Practice Song" (copies), plus `player.json`:
    `{"schema_version": 1, "source": "stem:guitar", "tempo": {"beats_per_bar": 4, "segments": [{"start_ms": 500.0, "bpm": 120.0}, {"start_ms": 10500.0, "bpm": 90.0}]}, "speed": 1.0, "metronome": {"on": false, "count_in": false}, "loops": [{"id": "loop-1", "start": {"tick": 64}, "end": {"tick": 192}, "repeat": true, "count_in": false}, {"id": "loop-2", "start": {"tick": 384}, "end": {"tick": 448}, "repeat": false, "count_in": true}], "current_loop": "loop-1"}`
    (loop-1 = 2500..6500 ms; loop-2 = 13166.667..15833.333 ms).
  - `...0305` "Missing Backing": type stem; stem vocals (440 Hz mono); a `backings` entry `backing` whose file is
    absent.
  - `...0306` "Missing Audio": type backing; `audio` "gone.flac", file absent (a track with nothing to play).
- done when: the script succeeds twice and the second run changes nothing (`git status --porcelain` clean after
  committing the first); `ffprobe` confirms rates/channels/durations; every JSON file parses
  (`python3 -m json.tool`); `npm test` passes.

### Task 2: Reserve the file name `player.json`
- routine: yes
- files: `src/calliope-gui/src/fsutil.rs`
- does: in `validate_file_name`, directly after the `if name == "track.json" { ... }` block, add
  `if name == "player.json" { return Err("\"player.json\" is reserved".into()); }`. In the test that loops over bad
  names (`for bad in ["..", ".", "a/b", ".x", "track.json", ...]`), add `"player.json"` to the array, and next to
  `assert!(validate_file_name("track.json.bak").is_ok());` add `assert!(validate_file_name("player.json.bak").is_ok());`.
- done when: `cargo test -p calliope-gui fsutil` passes and `npm test` passes.

### Task 3: Tempo map (`tempo_map.rs`)
- routine: no
- files: `src/tempo_map.rs` (new), `src/main.rs` (mod)
- does: 2.3: `TempoMap { beats_per_bar, segments }`, `TICKS_PER_BEAT = 16`, tick bases, `ms_at`, `tick_near`,
  `bpm_at`, `segment_at`, `beats_in`, `is_multi`, `set_bpm_at` (anchor, snap, replace/insert/remove, merge),
  `validate_bpm`, and `resnap(boundary, old, new) -> Boundary` for loops.
- done when: module tests pass, at least: one segment 120 BPM at 500 ms: `ms_at(64) == 2500`, `tick_near(2510) == 64`,
  `tick_near(2515.7) == 65`, `ms_at(-16) == 0`; the "With Tempo" map gives `T_1 == 320`, `ms_at(384)` within 1e-6 of
  13166.667, `bpm_at(10499) == 120`, `bpm_at(10500) == 90`; `beats_in(250..2000)` = 500, 1000, 1500 with the accent on
  500 only; `set_bpm_at` cases: empty+120 at 1234 -> one segment at 1234 exactly; existing 120 grid + 90 at 10510 ->
  segment at 10500 (snapped); 100 typed at 5000 with a change at 10500 -> new segment 5000..10500 only; 0 at 12000 ->
  removes the 90 segment; 0 on the only segment -> empty; equal neighbours merge; 19.9 and 300.1 refused; `resnap`
  keeps a boundary's time within half a tick and converts tick <-> ms when the map becomes empty / non-empty.

### Task 4: Player state and `player.json` (`player_state.rs`), export
- routine: no
- files: `src/player_state.rs` (new), `src/main.rs`, `src/repository.rs` (export copies `player.json` when present),
  `src/repository_tests.rs`
- does: 2.4 and 2.5: types with serde; `load(dir) -> (PlayerState, Option<note>, writable)` (lenient, unknown fields
  kept, newer schema read-only); `save(dir, &state)` via `fsutil::write_atomic`, refusing to create the folder;
  loop operations (`add`, `remove`, `select`, `mark`, `set`, `flags`; sorting by start with the current id kept;
  swap/refuse rules; 99 max; default lengths); `apply_tempo_change(new_map)` re-snapping every loop;
  `choose_source(saved, sources)` (the default rule of 2.2).
- done when: module tests pass, at least: the "With Tempo" fixture file round-trips with unknown fields kept;
  invalid JSON -> defaults + note; a bad loop is dropped and the rest kept; `schema_version: 2` -> read-only (save
  refused, file unchanged); save into a deleted folder fails and creates nothing; `+` at 3000 ms in BPM mode (120 at
  500) -> start tick 80, end tick 336; `>` after the end swaps; loops stay sorted and the current id survives;
  `choose_source` picks `backing:backing`, then `audio`, then the first stem, never `original`, and a stale saved key
  falls back; a repository test shows Export copies `player.json` next to `track.json` and still works without it.

### Task 5: Sources, decoding and peaks (`player_source.rs`)
- routine: no
- files: `src/player_source.rs` (new), `src/main.rs`
- does: 2.2: `sources(&TrackRecord)`, `decode` (FLAC via `stem_audio`, else ffmpeg pipe through
  `calliope_lib::process` with cancel; messages for a missing ffmpeg and a failed decode naming the file),
  `peaks(&Pcm, 2048)`.
- done when: module tests pass on copies of `library-player`: "Practice Song" lists `backing:backing`, `original`,
  `stem:vocals`, `stem:guitar` with labels "Backing", "Original mix", "Vocals", "Guitar"; "Missing Backing" lists only
  `stem:vocals`; "Missing Audio" lists nothing; "MP3 Backing" decodes through the real local ffmpeg to 44100 Hz stereo, about 20 s (+/- 0.1 s); a FLAC
  decodes to the exact samples `stem_audio::decode_i16` gives; peaks of the "Practice Song" backing are about 0.2 in
  the first quarter and about 0.9 in the second (+/- 0.02); a cancelled ffmpeg decode returns promptly with an error
  and leaves no child process.

### Task 6: Metronome click and scheduling (`metronome.rs`)
- routine: no
- files: `src/metronome.rs` (new), `src/main.rs`
- does: 2.6: `Click::new(rate)` (two pre-rendered sounds), `count_in_plan(bpm, rate) -> CountIn {clicks: [u64; 3],
  start: u64}`, `place(beats: &[(u64, bool)], block_start, block_len, speed) -> iterator of (offset, accent)`.
- done when: module tests pass: at 22050 Hz and 120 BPM the count-in clicks are at 0, 11025, 22050 and audio starts
  at 33075; the click peak is 0.5 and has decayed below 0.01 after 30 ms; accent and normal clicks differ in dominant
  frequency (2 kHz / 1.5 kHz, checked with a Goertzel filter in the test); `place` returns the beats inside a block
  only, at speed 1.0 at exact offsets.

### Task 7: Player engine core (`player.rs`): open, play, seek, source switch, persistence
- routine: no
- files: `src/player.rs` (new), `src/player_tests.rs` (new, `#[cfg(test)] #[path]`), `src/main.rs`
- does: 2.7 without metronome/count-in/loops: `Engine` (`apply`/`render`, fades, ended flag), command and return
  channels, status atomics; `PlayerManager` (ticketed open off the lock, `OutputRequest` pattern, deferred drops,
  `play`, `pause`, `seek`/`nudge` with the 0.3 s rule, `skip`, `select_source` keeping the position, `leave`,
  `tick(now)` with events and the 500 ms debounced save, `close` flushing), the types of 2.9, the Rust log lines.
- done when: tests pass with `ManualBackend` on copies of `library-player`: opening "Practice Song" gives 4 sources,
  `source == backing:backing`, 20000 ms; after `play` the pulled frames equal the decoded backing samples exactly
  (outside the 3 ms fade-in); `pause` gives zeros and a frozen position; `seek` while playing is silent until
  `tick(+300ms)`; `skip(-5000)` at 2 s gives 0; `select_source("stem:guitar")` at 4 s continues with guitar samples
  from 4 s; the end of the track stops at 0; a change marks dirty and `player.json` appears after `tick(+500ms)`, not
  at `+499ms`; `close` writes at once; reopening restores the source; a second `open` supersedes a running one.

### Task 8: Engine: metronome, count-in and loops
- routine: no
- files: `src/player.rs`, `src/player_tests.rs`
- does: 2.4 and 2.6 inside the engine and manager: grid upload on tempo change, metronome clicks, the count-in
  rules (main Play, loop Play, repeats), loop repeat with fades, loop one-shot end, loop Play/Stop, `loop_playing`,
  Prev/Next jumps, `set_bpm` (tempo change + loop re-snap + grid upload), `set_metronome`, the speed actions
  returning "not available yet".
- done when: tests pass with `ManualBackend` on "With Tempo" (22050 Hz): metronome on from 0 ms -> clicks at the
  frames for 0, 500, 1000, 1500 ms (exact onset frames; accent at 500 and 2500, not at 0, which is beat 4 of bar 0); Count In on + Play at 0 -> clicks at
  output frames 0, 11025, 22050, silence until 33075, then the source from frame 0; loop-1 Play with Loop On -> after
  the loop length the output continues from 2500 ms (wrap counter 1, `player wrap` logged), no count-in; loop-2 Play
  (loop count-in on) -> a count-in at 90 BPM before the first pass and before every repeat; loop with Loop On off ->
  stops at its end, position back to its start, `loop_playing` false; main Play inside a repeating loop wraps at its
  end; `set_bpm(100)` at 3000 ms -> loops keep their times within half a tick; metronome refused on a track without
  tempo; the callback never allocates (counting allocator in the test, as the Editor tests do).

### Task 9: IPC commands, shared backend, ticker, exit
- routine: no
- files: `src/player_ipc.rs` (new), `src/gui.rs`, `src/ipc.rs` (`EditorState::new` takes the shared backend; Editor
  Play pauses the Player), `src/editor.rs` (`EditorManager::new` takes `Arc<dyn OutputBackend>`), `build.rs`,
  `capabilities/main.json`, `src/ui/lib/ipc.ts`, `src/ui/lib/ipc-player.test.ts` (new), `src/ui/lib/fixture-tracks.ts`
  (player fixture records)
- does: the commands of 2.9 as thin async wrappers; `PlayerHost` (wrapper around `PlayerManager`, like `EditorState`) managed with the tools' ffmpeg path; the ticker calls
  it; `RunEvent::Exit` pauses, closes and flushes `player.json` (2 s cap); Player Play stops the Editor; TS types and
  wrappers (`openPlayer(id, onEvent)`, ...).
- done when: the static command-list test in `tests/frontend.rs` passes; `ipc-player.test.ts` checks each wrapper's
  command and argument names with `mockIPC`; existing Editor tests pass unchanged in behaviour; `npm test` passes.

### Task 10: Bar and BPM formatting
- routine: yes
- files: `src/ui/lib/bar-format.ts` (new), `src/ui/lib/bar-format.test.ts` (new), `src/ui/lib/bpm-format.ts` (new),
  `src/ui/lib/bpm-format.test.ts` (new)
- does:
  - `formatBar(tick: number, beatsPerBar: number): string`: `perBar = 16 * beatsPerBar`;
    `bar = Math.floor(tick / perBar) + 1`; `rest = tick - (bar - 1) * perBar`; `beat = Math.floor(rest / 16) + 1`;
    `sub = rest % 16`; returns `${bar}.${beat}.${sub}`.
  - `parseBar(text: string, beatsPerBar: number): number | null`: trim; match `^(-?\d+)\.(\d+)(?:\.(\d+))?$`; beat must be
    1..beatsPerBar and sub 0..15 (missing sub = 0), else null; returns `(bar - 1) * 16 * beatsPerBar + (beat - 1) * 16 + sub`.
  - `formatLoopTime(ms: number): string`: `r = Math.round(Math.max(ms, 0))`; `m = Math.floor(r / 60000)`,
    `s = Math.floor(r / 1000) % 60`, `f = r % 1000`; returns `${m}:${pad2(s)}.${pad3(f)}` (zero-padded to 2 and 3 digits).
  - `parseLoopTime(text: string): number | null`: trim; `^(\d+):([0-5]\d)(?:\.(\d{1,3}))?$` -> `(m*60+s)*1000 + frac`,
    where the fraction digits are right-padded with zeros to 3 digits; else fall back to `parsePosition` from
    `./time-format`; else null.
  - `formatBpm(bpm: number): string`: `0` -> `0`; integers without decimals; otherwise one decimal (`92.5`).
  - `parseBpm(text: string): number | null`: trim; `^\d+(?:\.\d)?$` else null; value 0, or 20..300 inclusive, else null.
- done when: `npx vitest run src/ui/lib/bar-format.test.ts src/ui/lib/bpm-format.test.ts` passes with at least these
  cases, and `npm run check` passes:
  - `formatBar`: (0,4) `1.1.0`; (64,4) `2.1.0`; (72,4) `2.1.8`; (16,4) `1.2.0`; (-16,4) `0.4.0`; (48,3) `2.1.0`;
  - `parseBar`: (`2.1.8`,4) 72; (`1.1`,4) 0; (`0.4.0`,4) -16; null for `1.5.0` (beats 4), `1.1.16`, `a.b`, ``;
  - `formatLoopTime`: 0 `0:00.000`; 67250 `1:07.250`; 13166.667 `0:13.167`; -3 `0:00.000`;
  - `parseLoopTime`: `1:07.250` 67250; `1:07.25` 67250; `1:07` 67000; `75.5` 75500; null for `1:7`, `x`;
  - `formatBpm`: 0 `0`; 120 `120`; 92.5 `92.5`; `parseBpm`: `120` 120; `92.5` 92.5; `0` 0; null for `19.9`, `300.1`,
    `12a`, `92.55`, ``.

### Task 11: Waveform geometry
- routine: yes
- files: `src/ui/lib/waveform-geometry.ts` (new), `src/ui/lib/waveform-geometry.test.ts` (new)
- does:
  - `columns(peaks: number[], width: number): number[]`: for `width <= 0` or empty peaks return `[]`; for each column
    `x` in `0..width-1`: `a = Math.floor(x * peaks.length / width)`, `b = Math.max(a + 1, Math.floor((x + 1) * peaks.length / width))`;
    value = max of `peaks[a..b)` (b capped at `peaks.length`).
  - `xOf(ms: number, durationMs: number, width: number): number`: `durationMs <= 0` -> 0; else
    `Math.round(Math.min(Math.max(ms, 0), durationMs) / durationMs * (width - 1))`.
  - `loopMarks(loops: {start_ms: number; end_ms: number}[], durationMs: number, width: number): {x1: number; x2: number}[]`:
    `xOf` of each start and end, same order.
- done when: `npx vitest run src/ui/lib/waveform-geometry.test.ts` passes with at least: `columns([0.1,0.5,0.2,0.9], 2)`
  = `[0.5, 0.9]`; `columns([0.1,0.5], 4)` = `[0.1, 0.1, 0.5, 0.5]`; `columns([], 10)` = `[]`; `xOf(5000, 20000, 801)` = 200;
  `xOf(-1, 20000, 801)` = 0; `xOf(30000, 20000, 801)` = 800; `loopMarks([{start_ms: 2500, end_ms: 6500}], 20000, 801)` =
  `[{x1: 100, x2: 260}]`; and `npm run check` passes.

### Task 12: Player view layout, collapsible tree, select component
- routine: no
- files: `src/ui/views/PlayerView.svelte`, `src/ui/views/PlayerView.test.ts` (new),
  `src/ui/components/player/PlayerPane.svelte` (new, layout with placeholders for the sections),
  `src/ui/components/player/SourceSelect.svelte` (new), `src/ui/lib/components/ui/select/**` (new),
  `src/ui/components/library/TrackBrowser.svelte` (optional `collapsed` rail mode), `src/ui/lib/views.ts` (Player
  summary/feature text), `src/ui/lib/views.test.ts`, `src/ui/App.test.ts` if it expects the Player placeholder,
  `docs/licences.md` (select component in the copied-components row)
- does: 2.8 layout and Ctrl+L rail; the Player rescans on mount like the Editor; the dropdown component (large,
  keyboard-operable, both themes); the tablature placeholder; the `player tree collapsed=` log line.
- done when: vitest: the Player renders the shared tree and selecting a track there sets `lib.selectedId`; Ctrl+L
  hides the tree and shows the rail with the "Library" button, Ctrl+L or the button restores it, and `#view-heading`
  exists in both states; the select lists given options and reports a change; the static CSP/style tests pass;
  `npm test` passes.

### Task 13: Player frontend state
- routine: no
- files: `src/ui/lib/player-state.svelte.ts` (new), `src/ui/lib/player-state.test.ts` (new), `src/ui/App.svelte`
  (`leavePlayer()` when the view changes away from `player`)
- does: 2.10 state and actions; follows the selection like the Editor (open/close, stale snapshots ignored,
  `superseded` ignored); coalesced seeks; events applied; frontend log lines of 2.9; re-attach after reload.
- done when: vitest with `mockIPC`: no selection -> `inactive`, no `open_player`; a playable track -> `open_player`,
  `active`, `player active ... source=backing:backing sources=4`; switching tracks closes/opens and ignores a late
  snapshot; `selectSource` calls `player_select_source` and logs; a `transport` event updates position and `bpm`;
  leaving the view while playing calls `player_leave` and logs `player leave pause`; 20 rapid seeks keep at most 2
  calls in flight with the last value last; `npm test` passes.

### Task 14: Control bar sections (BPM, Metronome, Playback, Loops)
- routine: no
- files: `src/ui/components/player/{BpmSection,MetronomeSection,PlaybackSection,LoopsSection}.svelte` (new),
  `src/ui/components/player/ControlBar.test.ts` (new), `src/ui/components/editor/TimeField.svelte` (props),
  `src/ui/components/editor/TimeField.test.ts`, `src/ui/components/editor/MixLane.svelte` (uses the props),
  `src/ui/components/player/PlayerPane.svelte`
- does: 2.8 sections with the enables, labels, `aria-pressed` push buttons, BPM field editing, bar boxes in both
  modes, read-only loop slider, disabled `-`/`+`/`Reset`/`Detect` with the hint, "V" indicator.
- done when: vitest with `mockIPC` and the "With Tempo" / "Practice Song" records: BPM shows `120`, the V indicator is
  checked for "With Tempo" and not for one segment; typing `90` + Enter calls `player_tempo` with `set-bpm` 90;
  `19` marks the field invalid with no call; metronome buttons are disabled in timestamp mode; section box `1/2`;
  `bar1` shows `2.1.0` and `bar2` `4.1.0` for loop-1, and `0:02.500` style in timestamp mode; typing `3.1.0` in
  `bar1` sends `set` with `{tick: 128}`; `>`, `<`, `+`, `-`, Prev, Next, Play/Stop, Loop On, Count-In send the right
  `player_loop` actions; the loop slider is not focusable and reflects position 4500 ms as 50 %; Play reads Pause
  while `counting_in`; Rew/FF send `player_skip` -5000/5000; the Editor's TimeField tests still pass; `npm test`
  passes.

### Task 15: Waveform view
- routine: no
- files: `src/ui/components/player/WaveformView.svelte` (new), `src/ui/components/player/WaveformView.test.ts` (new),
  `src/ui/components/player/PlayerPane.svelte`
- does: 2.10: canvas drawing with `waveform-geometry`, device-pixel-ratio aware, `ResizeObserver` redraw, theme colours
  read from CSS variables, loop marks (current loop brighter), playhead; `role="img"` with an `aria-label` that names
  the loop count.
- done when: vitest (canvas context stubbed): drawing is requested with `columns(peaks, width)` for the measured
  width, a resize redraws with the new width, loop marks are drawn at `loopMarks(...)` positions, and the label says
  "Waveform, 2 loops"; `npm test` passes. (Pixels are checked in tasks 16-17.)

### Task 16: Player keyboard shortcuts
- routine: no
- files: `src/ui/views/PlayerView.svelte`, `src/ui/views/PlayerView.test.ts`
- does: 2.8 keys with the focus rules.
- done when: vitest: Space toggles play only outside text fields/sliders/buttons/select/tree; `b`/`e` send `mark`
  start/end; arrows send `select` -1/+1 outside the tree, and move in the tree when it has focus; `L` toggles
  `repeat`; `[`/`]` do nothing in increment A; Ctrl+L works with the focus anywhere in the view; `npm test` passes.

### Task 17: GUI end-to-end tests, visual review, docs
- routine: no
- files: `tests/gui_player_e2e.rs` (new), `tests/common/mod.rs` (`start_player` helper copying `library-player` with
  `CALLIOPE_E2E_AUDIO=capture:<target/...wav>`; WAV reading and band-energy onset helpers moved here from
  `tests/gui_editor_e2e.rs`), `package.json` (`test:gui` adds `--test gui_player_e2e`), `docs/ui.md` ("Decided by the
  team" entries for the Player), `src/calliope-gui/README.md` (Player usage, keys), `docs/architecture.md` (only
  deviations)
- does: on display `:1`, window floated at 1280x800, keyboard first (data-testid bounds or verified coordinates for
  the mouse), asserting on log lines, `player.json` on disk and the capture WAV:
  1. Alt+5; select "Stems Only" -> `player active ... source=stem:bass`; Ctrl+L -> `player tree collapsed=true`
     (screenshot), Ctrl+L -> false.
  2. "Practice Song" -> `source=backing:backing sources=4`; choose "Guitar" in the dropdown -> `player source
     key=stem:guitar`, and within 1 s `player.json` has `"source": "stem:guitar"`.
  3. Play 2 s -> capture has 660 Hz, not 330 Hz; Pause -> position stable.
  4. Time field `0:01.0`, BPM `120` + Enter -> `player bpm value=120 position=1000`, file has one segment at 1000.
  5. Metronome On, Play 3 s -> click onsets in the capture (1.5/2 kHz band) every 500 ms +/- 5 ms; Off.
  6. Count In On, Play from a pause -> three clicks 500 ms apart, then the guitar tone starts 1500 ms after the first
     click (+/- 10 ms) with no click there.
  7. `+`, then `>`/`<` at chosen positions, Loop On, loop Play for 2 passes -> `player wrap loop=1` twice; Loop On
     off, loop Play -> stops at the end (`playing=false`); loop Count-In on -> `player wrap ... count_in=true`.
  8. Close gracefully, restart with the same dirs, select "Practice Song" -> `source=stem:guitar mode=bpm loops=1`.
  9. "MP3 Backing" -> plays (`player open ... source=audio rate=44100`); "Missing Backing" -> `sources=1
     source=stem:vocals`; "Missing Audio" -> `player inactive id=...0306` and the message.
  10. Alt+1 while playing -> `player leave pause` and Rust `playing=false`.
  Every test asserts no `csp-violation` and that the backend is never `cpal`. Visual review: screenshots at 1280x800
  and 1024x640 (tree shown and collapsed), dark and light, of the active Player with loops on the waveform, the
  inactive state and a count-in; check readability at 1-2 m, control sizes, focus rings, the amber marks, nothing cut
  off; fix what is found and record the UI decisions.
- done when: `DISPLAY=:1 npm run test:gui` passes three times in a row (all suites); the reviewed screenshots are in
  `target/gui-shots/player-*.png` and listed in the task log; `npm test` passes.

### Increment B (after Q3)

### Task 18: Time-stretch dependency and licence record
- routine: no
- files: `src/calliope-gui/Cargo.toml`, `Cargo.lock`, `docs/licences.md`, `src/calliope-gui/README.md` (build
  prerequisite `clang` if the chosen crate uses bindgen), `docs/architecture.md` (Build and run prerequisites)
- does: add the crate chosen in Q3; record its licence and the bundled C++ library's licence from the package
  metadata and the vendored LICENSE; regenerate nothing else.
- done when: `cargo build -p calliope-gui` succeeds; the licence-record test in `tests/frontend.rs` passes; the third-
  party notices test passes; `npm test` passes.

### Task 19: Stretcher (`stretch.rs`)
- routine: no
- files: `src/stretch.rs` (new), `src/main.rs`
- does: 2.12 trait and implementation; allocation only in `new`.
- done when: module tests pass: a 440 Hz stereo tone at speed 0.8 and 1.25 keeps its dominant frequency within 1 %
  (Goertzel scan); 10 s of input at speed 0.8 gives 12.5 s of output (+/- latency); after `prime` the first output
  frames are not silent; no allocation in `process` (counting allocator).

### Task 20: Engine and state: speed
- routine: no
- files: `src/player.rs`, `src/player_tests.rs`, `src/player_state.rs`
- does: 2.12 in the engine (feed ratio, position latency, priming, clicks at `/speed`, count-in at `bpm * speed`) and
  the `speed-step`/`speed-reset` actions; typing a BPM resets the speed; speed persisted.
- done when: tests pass: speed 1.0 is still bit-exact; at 115/120 the position advances 0.9583 s per output second
  (+/- 1 %); metronome clicks at 115 BPM spacing in output time; `-` at 120 gives `bpm 115`; `Reset` gives 1.0 and the
  correct BPM at the position (also in the second tempo segment); without a tempo `-` gives speed 0.95; limits
  0.50/1.50; a loop repeats correctly at speed 0.8.

### Task 21: Frontend and GUI for speed
- routine: no
- files: `src/ui/components/player/BpmSection.svelte`, `src/ui/components/player/ControlBar.test.ts`,
  `src/ui/views/PlayerView.svelte` (`[`/`]`), `tests/gui_player_e2e.rs`, `docs/ui.md`
- does: enable `-`/`+`/`Reset`, the speed readout, the keys; an e2e step: BPM 120, `-` x4 -> `player bpm`/transport
  shows 100, and the capture keeps 660 Hz (+/- 1 %) while the position advances at 100/120.
- done when: vitest for the enables and calls; `DISPLAY=:1 npm run test:gui` passes three times in a row; `npm test`
  passes.

### Increment C (after Q4; recommended as its own spec, outline only)

### Task 22: Stub beat tracker and fixtures
- routine: no
- files: `tests/support/stub-beat-tracker` (bash), `tests/fixtures/import/beats/*.json`
- does: canned beat/downbeat lists (constant 120 BPM; 120 then 90 BPM), modes ok/fail/slow like the stub separator.
- done when: `bash -n` passes and the script writes the canned JSON for each mode; `npm test` passes.

### Task 23: `calliope-stems` beats endpoints and client
- routine: no
- files: `src/calliope-stems/src/*.rs`, `src/calliope-stems/tests/conformance.rs`, `src/calliope-lib/src/stems_api.rs`,
  `src/calliope-lib/src/stems_client.rs`, `src/calliope-stems/README.md`, `src/calliope-stems/separators/` (real
  tracker adapter, syntax-checked only)
- does: 2.13 server and client, `--beat-tracker`, `tasks` in health.
- done when: the conformance suite covers the new endpoints with raw HTTP and the client; a server without
  `--beat-tracker` reports no `beats` task; `npm test` passes.

### Task 24: Tempo segmentation from beats
- routine: no
- files: `src/tempo_map.rs` (or `src/calliope-lib/src/beats.rs`)
- does: 2.13 segmentation.
- done when: unit tests: the 120 BPM list gives one segment at the first downbeat; the 120->90 list gives two segments
  with the change within one beat of the truth; jittered beats (+/- 10 ms) still give one segment.

### Task 25: Detect in the Player
- routine: no
- files: `src/player.rs`, `src/player_ipc.rs`, `build.rs`, `capabilities/main.json`, `src/ui/lib/ipc.ts`,
  `src/ui/components/player/BpmSection.svelte`, `tests/gui_player_e2e.rs` (network-isolated, server with the stub)
- does: `player_detect`/`cancel_player_detect`, the job with progress, result applied (loops re-snapped, speed reset).
- done when: tests with the real server and stub tracker on 127.0.0.1 inside `unshare -rn`; vitest; GUI suite three
  times green; `npm test` passes.

## 4. Test strategy

- **Whole headless suite:** `cd src/calliope-gui && npm test` (svelte-check, vitest, vite build,
  `cargo test --workspace`, clippy with `-D warnings`).
- **GUI suite:** `cd src/calliope-gui && DISPLAY=:1 npm run test:gui` (built with `e2e-hooks`).
- **Audio hardware stand-ins** (no test ever opens the real device or makes a sound):
  - `ManualBackend` in unit tests: the test pulls frames from the real render callback and compares them sample by
    sample (source copy, fades, click onsets, count-in timing, loop repeats).
  - `Engine` is also tested directly (`apply`/`render`) without any backend.
  - `NullBackend` with `CALLIOPE_E2E_AUDIO=capture:<abs path>` in GUI tests; the test analyses the WAV: tone presence
    per frequency (330/440/550/660 Hz sources) and click onsets in the 1.5-2 kHz band.
  - `CpalBackend` is never constructed by a test (existing static test).
- **ffmpeg** (non-FLAC sources) is the real local tool on the generated MP3 fixture (no network).
- **Pure logic:** tempo map math and snapping, loop operations and re-snapping, `player.json` lenient parsing, source
  choice, peaks, click synthesis, count-in plan, debounce with injected `Instant`s (no sleeps for timing rules).
- **Frontend:** vitest + `mockIPC` for state, components, formatting, keys; `transport` events simulated through the
  channel callback; canvas stubbed.
- **End-to-end:** the real binary on `:1` with temp XDG dirs and a copied `library-player` under `target/`; log lines,
  `player.json` on disk, capture WAV analysis, screenshots for review.
- **Edge-AI (increment C only):** the real `calliope-stems` with a stub beat tracker on 127.0.0.1 inside `unshare -rn`;
  never archserver, never the real `calliope-stems` on 0.0.0.0:8765 of this laptop, never the real model.
- **Time-stretch (increment B):** frequency and duration checks on synthetic tones; real listening quality is a
  manual check.

## 5. Deployment

- **Increments A and B:** single machine. Everything runs in calliope-gui on the laptop (`npm run build:app`). No
  change on archserver. Increment B adds the build prerequisite of the chosen stretch crate (for
  `signalsmith-stretch`: a C++ compiler, already in `base-devel`, and `clang` for bindgen: `pacman -S clang`).
- **Increment C** (when built): the owner deploys by hand, as before:
  1. `cargo build --release -p calliope-stems` on the laptop, copy `target/release/calliope-stems` to archserver.
  2. On archserver, create the beat-tracker venv and copy the adapter script (README in `src/calliope-stems/`).
  3. Add `--beat-tracker <path to adapter>` to `deploy/calliope-stems.service` (`ExecStart`), then
     `systemctl --user daemon-reload && systemctl --user restart calliope-stems`.
  4. Check `curl http://archserver:8765/v1/health` lists `"tasks": ["stems", "beats"]`.

## 6. Manual checks (owner, laptop with real audio)

1. `npm run build:app`, run `target/release/calliope-gui`, press Alt+5, select a real stem track with a saved
   backing: the dropdown shows "Backing", the waveform shows the song's loud and quiet parts, Play plays the backing
   on the default output and the time and slider move smoothly.
2. While playing, switch the dropdown to Guitar, then back to Backing: playback continues at the same place, with no
   long gap.
3. Pause on the first downbeat, type the song's BPM and press Enter. Turn the metronome On and play: the ticks fall on
   the beat, the accent on beat 1, for the whole song (or until a tempo change).
4. Turn Count In on, pause and press Play: three ticks, then the music starts on the fourth beat, in time.
5. Make a loop around a chorus with `>`/`<` (or `b`/`e`) while listening, turn Loop On and press the loop's Play: it
   repeats without a click or a gap at the joint. Turn the loop's Count-In on: a count-in before each pass.
6. Press `+` twice more, use Prev/Next (or arrows): the position jumps at once; the amber marks on the waveform match
   what you hear.
7. Close the app and start it again: the track, source, BPM and loops are back. Look at `player.json` in the track
   folder: it is readable.
8. Select an old plain backing track whose audio is MP3: it plays (needs ffmpeg).
9. Ctrl+L collapses the tree and gives the waveform more width; Ctrl+L brings it back.
10. Switch to the Library while playing: playback pauses; back in the Player it resumes from the same place.
11. From 1-2 m in a dim room: BPM, time, section number and the loop marks are readable; the important actions work
    from the keyboard (Space, b, e, arrows, L).
12. (Increment B) `-` four times at 120 BPM: the song plays at 100 BPM with the same pitch and acceptable sound
    quality; `Reset` returns to normal speed.
13. (Increment C) `Detect` on a song with a known tempo change: the BPM field and the V marker show it.

## 7. Acceptance mapping

The spec has no acceptance criteria yet. Proposed criteria (owner to confirm, Q1), each mapped to tasks and tests:

| #  | Proposed criterion (Given / When / Then)                                                                                                                                                                                                                             | Req.    | Tasks        | Tests                                         |
|----|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------|--------------|-----------------------------------------------|
| 1  | Given the Player view, when the user presses Ctrl+L, then the tree collapses to a narrow rail; Ctrl+L again restores it and the selection is unchanged                                                                                                               | 1       | 12, 16, 17   | vitest PlayerView; e2e step 1                 |
| 2  | Given a track selected in the Library, when the user opens the Player, then the same track is selected and loaded                                                                                                                                                    | 1, 2    | 12, 13       | vitest; e2e                                   |
| 3  | Given no track or a track without audio is selected, then the player pane shows an inactive message and its controls are disabled                                                                                                                                    | 2       | 12, 13, 14   | vitest; e2e step 9 ("Missing Audio")          |
| 4  | Given a track with audio is selected, then the pane shows the dropdown, the tablature area, the waveform and the control bar with BPM, Metronome, Playback and Loops                                                                                                 | 2, 5-8b | 12, 14, 15   | vitest; screenshots                           |
| 5  | Given a stem track with a backing variant opened for the first time, then the dropdown lists the backings, the original mix (if kept) and the stems, and selects the backing                                                                                         | 4       | 5, 4, 7      | Rust sources/choose_source; e2e step 2        |
| 6  | Given a stem track without a backing opened for the first time, then the first stem is selected                                                                                                                                                                      | 4       | 4, 5         | Rust; e2e step 1                              |
| 7  | Given the user changes the dropdown, then `player.json` next to `track.json` records it within 1 s, and the choice is restored when the track is selected again or the app restarts                                                                                  | 3, 4    | 4, 7, 9      | Rust debounce; e2e steps 2, 8                 |
| 8  | Given playback is running, when the user changes the dropdown, then the playback stops                                                                                                                                                                                             | 4, 13   | 7            | Rust exact samples after the switch; manual 2 |
| 9  | Given the player pane, then the tablature area is an empty placeholder taking the remaining height                                                                                                                                                                   | 6       | 12           | vitest; screenshots                           |
| 10 | Given a selected track, when the user presses Play, then audio plays, Play reads Pause, and the slider and the `M:SS.T` field follow the position; Pause holds the position                                                                                          | 11      | 7, 13, 14    | Rust; vitest; e2e step 3                      |
| 11 | Given the time field, when the user types a time within the track, then playback jumps there; outside, the field is marked invalid                                                                                                                                   | 11      | 14           | vitest TimeField                              |
| 12 | Given any position, when the user presses Rew or FF, then the position moves 5 s back or forward at once, clamped to the track                                                                                                                                       | 11      | 7, 14        | Rust skip; vitest                             |
| 13 | Given a track without tempo, then the BPM field shows 0, the V marker is unchecked, the loop boxes show timestamps and Metronome and Count In are disabled                                                                                                           | 14, 15  | 3, 14        | Rust; vitest                                  |
| 14 | Given a track without tempo, when the user types 120 at position p, then the whole track has 120 BPM with bar 1 at p, and it is saved                                                                                                                                | 14      | 3, 8, 14     | Rust set_bpm_at; e2e step 4                   |
| 15 | Given a 120 BPM track, when the user types 90 at a later position, then 90 applies from that position (snapped to the nearest 1/64) to the end or the next change, the V marker is checked, and the BPM field shows 120 before and 90 after the change while playing | 14      | 3, 8, 14     | Rust; vitest; fixture "With Tempo"            |
| 16 | Given tempo changes, when the user types 0 at a position, then the tempo in effect there is removed; removing the last one returns to timestamp mode with the loops kept at their times                                                                              | 14      | 3, 4         | Rust                                          |
| 17 | (Increment B) Given 120 BPM, when the user presses `-`, then the BPM field shows 115 and playback runs at 115/120 of the speed with the pitch unchanged; `+` goes up by 5; Reset returns to the correct BPM at the position                                          | 14      | 19-21        | Rust stretch/engine; e2e                      |
| 18 | (Increment C) Given the edge-AI server offers beat tracking, when the user presses Detect, then the detected tempo map (several tempos possible) replaces the current one                                                                                            | 14      | 22-25        | Rust + stub server; e2e                       |
| 19 | Given BPM mode and Metronome On, when playing, then a tick sounds on every beat of the tempo map, accented on beat 1; Off stops the ticks                                                                                                                            | 15      | 6, 8         | Rust onsets; e2e step 5                       |
| 20 | Given BPM mode and Count In on, when the user presses Play, then three ticks sound one beat apart and playback starts on the fourth beat without a tick                                                                                                              | 15      | 6, 8         | Rust exact frames; e2e step 6                 |
| 21 | Given a loop with Loop On, when it is played with Count In on, then the count-in plays before the first pass only; with the loop's Count-In on, before every pass                                                                                                    | 15      | 8            | Rust; e2e step 7                              |
| 22 | Given the Loops section, when the user presses `+`, then a new loop starting at the position becomes current and the section box shows its number and the count; `-` removes the current loop                                                                        | 16      | 4, 8, 14     | Rust; vitest                                  |
| 23 | Given several loops, when the user presses Prev or Next, then the previous/next loop becomes current and the position jumps to its start at once                                                                                                                     | 16      | 4, 8, 14     | Rust; vitest                                  |
| 24 | Given BPM mode, when the user presses `>` or `<`, then the loop start/end is set to the nearest 1/64 position (stored as a division, shown as bar.beat.tick); in timestamp mode the exact time is stored and shown as `M:SS.mmm`                                     | 16      | 3, 4, 10, 14 | Rust; vitest                                  |
| 25 | Given a current loop, when the user presses its Play, then the position jumps to the loop start and the section plays; with Loop On it repeats, otherwise it stops at the end; the button reads Stop while playing and Stop stops (no pause)                         | 16      | 8, 14        | Rust; e2e step 7                              |
| 26 | Given a current loop, then the small slider shows the position inside the loop and cannot be moved by the user                                                                                                                                                       | 16      | 14           | vitest                                        |
| 27 | Given the window is resized, then the waveform fills the width and is redrawn; loop sections appear as amber vertical marks joined by an amber bar                                                                                                                   | 17      | 11, 15, 17   | vitest; screenshots                           |
| 28 | Given any player setting is changed (source, BPM, metronome, count-in, loops and their flags, current loop), then it is on disk within 1 s without a Save button and restored after reselecting the track or restarting                                              | 3, 13   | 4, 7, 8      | Rust; e2e step 8                              |
| 29 | Given playback is running, when the user leaves the Player view, then playback pauses at its position                                                                                                                                                                | (Q11)   | 7, 13        | vitest; e2e step 10                           |

## 8. Open questions

1. **Acceptance criteria.** The spec's section is the template. Please confirm or edit the 29 criteria proposed in
   section 7 (they also pin the behaviour choices below). Also: the spec has two requirements numbered 8; the plan
   calls the second one 8b. *Recommendation:* accept section 7 as the criteria.
2. **Split into increments.** One build covering everything is larger than any feature so far and has two external
   dependencies that need your decisions. *Recommendation:* build **A** now (view, dropdown, playback, waveform,
   manual tempo map with the V marker, metronome, count-in, loops, persistence; `-`/`+`/`Reset`/`Detect` visible but
   disabled), then **B** (speed change keeping the pitch, tasks 18-21) as an increment, then **C** (Detect on the
   edge-AI server) as its own spec `gui-play-backing-track-detect.md`. If you answer Q3 now, A and B can be built
   together.
3. **Time-stretch library (tempo change keeping the pitch).** Options checked:
   - `signalsmith-stretch` 0.1.3 (Rust binding, MIT per its repository; it compiles the Signalsmith Stretch C++
     library, MIT, with `cc`, and uses bindgen, so building needs `clang`). Good quality on music, small and
     permissive; the Rust binding is young (one maintainer).
   - Rubber Band: GPL-2.0-or-later or a paid licence. Not compatible with keeping Calliope permissive. Rejected.
   - SoundTouch: LGPL-2.1, and its Rust crates build it statically into the binary, which brings LGPL relinking
     obligations. Not recommended.
   - Bungee: MPL-2.0. Not recommended, given your rejection of MPL for `symphonia`.
   - Our own WSOLA in Rust: no dependency, but audibly worse on chords and drums.

   *Recommendation:* `signalsmith-stretch` behind our own `Stretcher` trait (switching later stays a one-file
   change), speed limited to 50-150 %, `clang` as a new build prerequisite.
4. **BPM detection.** Multi-tempo detection needs a beat-tracking model. *Recommendation:* "Beat This!" (CPJKU; code
   and weights listed as MIT, to be re-checked) run on archserver behind new additive `calliope-stems` endpoints
   (`/v1/beats`, 2.13), with the app turning beats into tempo segments; in its own spec after B. Alternative: a
   simple local single-tempo detector in Rust (no server, but no multi-tempo).
5. **Settings per track or per source?** Req. 13 says the controls "apply to the selected mix or stem". Stems and
   backings of a track are sample-aligned. *Recommendation:* one tempo map, one loop list, one speed and one metronome
   setting per **track**, shared by every source; only the dropdown choice is the source.
6. **BPM field, `-`/`+` and `Reset`.** *Recommendation:* typing a value sets the track's tempo from the position (and
   resets the speed to 100 %); `-`/`+` change the playback speed so the shown BPM moves by 5 (`[`/`]` do the same,
   instead of the UI guide's 5 %); `Reset` returns to 100 % speed, so the field shows the correct BPM at the position.
   **With no tempo (BPM 0)**: `-`/`+` change the speed in 5 % steps with a "Speed 95 %" readout, and `Reset` returns
   to 100 % (the field stays 0). Alternative: `-`/`+`/`Reset` disabled while the BPM is 0.
7. **Where the player state is stored.** *Recommendation:* `player.json` next to `track.json` in the track folder
   (2.5): written often without touching `track.json` (no Library save conflicts), ignored by older builds, included
   in exports, `player.json` reserved as a file name. Alternative: a `player` object inside `track.json` (rejected:
   every loop edit would change the track's revision and break a Library edit in progress). Note: when
   `gui-manipulate-backing-track` adds tempo to the track metadata (overview use case 4), the tempo map may move into
   `track.json` then.
8. **Time signature and the grid.** *Recommendation:* 4/4 for now (`beats_per_bar` stored, default 4, editable in a
   later feature); bar 1 starts where the first tempo was entered (or at the first detected downbeat); tempo changes
   snap to the nearest 1/64 of the current grid.
9. **The "V" marker.** *Recommendation:* a read-only check indicator that is on when the track has several tempos.
   Alternative: a dropdown listing the tempo sections (belongs to `gui-manipulate-backing-track`).
10. **Loop and count-in rules.** *Recommendation:* `Loop On` repeats the current loop whenever playback reaches its
    end from inside it (main Play too); with it off, the loop's Play plays the section once and stops at its start;
    the metronome's `Count In` counts in before main Play and before a loop's first pass; a loop's `Count-In` counts
    in before every pass of that loop (and the first); `-` removes a loop without a confirmation; a new loop is 4
    bars (or 10 s) long.
11. **Leaving the view, Ctrl+L.** *Recommendation:* leaving the Player pauses (position kept), unlike the Editor,
    which stops; selecting another track stops. Ctrl+L turns the tree into a 3.5 rem rail with one "Library" button,
    not remembered between runs, Player view only.
12. **Metronome sound.** *Recommendation:* our own synthesised click (2 kHz accent on beat 1, 1.5 kHz other beats,
    30 ms, -6 dBFS) mixed into the same output as the track after the speed change, so it is always in time; no
    separate volume control in this feature (MIDI clock comes with the MIDI features).
13. **Small behaviours.** *Recommendation:* Rew/FF jump 5 s per press; the dropdown lists the backings, "Original
    mix" (if kept) and the stems, and never picks the original automatically; the position is not saved (a track
    opens at 0:00.0).
14. **Settings placeholders.** `views.ts` credits this feature with the "Audio output" and "MIDI interface" settings,
    but the spec doesn't mention them. *Recommendation:* out of scope here (the default output device stays); the
    placeholders will name a later feature.
