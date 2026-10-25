# Plan: gui-first-beat (place the song's first beat where the music starts)

Spec: `specs/gui-first-beat.md` (owner answers 2026-10-25, including the last paragraph: the automatic
search is skipped on a track whose first beat is not at 0.0; "Find start" can still move it).
Base: `f1c29e9` on `spec/gui-first-beat`. Paths follow "Paths in this document" of `docs/architecture.md`:
`src/*.rs`, `src/ui/...` and `tests/...` are inside `src/calliope-gui/`.

Note on the spec's Context: it says "six ticks over 4 beats, the 5th silent". The code since the
2026-10-25 fix (`metronome.rs` `TICK_BEATS` / `LEAD_BEATS`, shared with `export_audio.rs`) is six ticks
over a **5-beat** lead, the 5th beat silent, which is also what the acceptance criteria say ("the 5-beat
lead"). This plan uses the code's 5 beats.

## 1. Summary

Calliope gets a small onset detector (`src/first_beat.rs`) that finds where the music starts, relative to
the track's loudest part, on the track's backing audio. The Player runs it once when a track with a tempo
opens with its first beat still at 0:00, and again on a new "Find start" button; the result moves the first
tempo section's start (bar 1, beat 1) there and is stored as `found_ms` on the first segment
(`player.json` schema 7). The first section's start bar becomes draggable (Editor and Player, 1/64-beat
steps, 0:00 up to one beat before the next section, readout = offset from the found position); a first
beat that is "set" (found or moved) dims the audio before it, keeps the Player's playhead and playback
from it, and makes the export drop it (file = 5-beat lead + the audio from the first beat on). Tracks
saved before this feature play and export exactly as today until their first beat is found or moved.

## 2. Design

### 2.1 Data model: the "set" first beat (`tempo_map.rs`, `player_state.rs`)

New optional field on `Segment`, meaningful only on the first segment:

```rust
/// First segment only: where "Find start" placed the first beat, or where the first beat was before its
/// first move by hand. Present = the first beat is *set*: the audio before `start_ms` is left out of
/// playback and export, and moves are counted from here.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub found_ms: Option<f64>,
```

JSON: `{"start_ms": 1200.0, "bpm": 120, "found_ms": 1200.0}` on `tempo.segments[0]`.

Why a segment field: it travels with the first segment through every existing edit (clone, BPM, flags,
names), exactly like `detected_ms` does for changes, and the frontend gets it in the snapshot's segments
with no new IPC field. Why "set = `found_ms` present" instead of a separate flag: one key carries both
facts the feature needs (the reference for the readout and "this first beat was placed deliberately"), and
its absence is exactly the state of every file written before this feature (requirement 6).

Rules (all in `TempoMap`, pure, unit-tested):

| Function | Behaviour |
|---|---|
| `first_beat_set() -> bool` | `segments[0].found_ms.is_some()` |
| `play_start_ms() -> f64` | `max(0, segments[0].start_ms)` when set, else `0.0` (also `0.0` on an empty map). The Player's floor and the export's cut |
| `needs_first_beat() -> bool` | non-empty map, `segments[0].start_ms == 0.0` exactly, `found_ms` absent: the automatic search may run (owner: skipped when the first beat is not at 0.0) |
| `first_beat_limits(end_ms) -> Result<(f64, f64), String>` | `(0.0, hi)`, `hi = (segments[1].start_ms or end_ms) − 60000/bpm0` (one beat of the first section's own tempo: every section keeps at least one beat, the existing invariant). Empty map: `Err("Type a BPM first")`; `hi < 0`: `Err("There is no room to move the first beat")` |
| `place_first_beat(ms, end_ms) -> Result<TempoMap, String>` | `start = clamp(ms, 0, hi)`, `found_ms = Some(start)` (so the readout is 0 right after Find start). If the start changed and `segments[1]` exists and does not start on a beat, it becomes a placed change (`on_beat = true`), as `move_change` does for the change it moves |
| `move_first_beat(steps, end_ms) -> Result<(TempoMap, i64), String>` | `ref = found_ms.unwrap_or(start_ms)`, `fine = fine_ms(bpm0)` (1/64 beat of the first section's own tempo: it has no left section). `nmin = floor((0 − ref)/fine)`, `nmax = ceil((hi − ref)/fine)` (`hi` infinite → no max); `applied = clamp(steps, nmin, nmax)`; `start = clamp(ref + applied·fine, 0, hi)`, so the extreme steps land exactly on 0:00 and on `hi` ("stops at 0:00 / just before the next section"). `found_ms = Some(ref)` (a never-found first beat gets its old position as the reference and becomes set). `segments[1]` becomes `on_beat` as above. No ±256 limit (owner: "anywhere from 0:00 up to the next section's start"). Returns the applied steps |
| `section_jump_ms(0, Start)` | returns `play_start_ms()` instead of `0.0` (Start / Home in the Player land on the first beat when it is set; unchanged otherwise) |
| `set_bpm_at(.., 0)` removing segment 0 | the new first segment also gets `found_ms = None` (as it already gets `detected_ms = None`, `on_beat = false`) |

Requirement 5 needs no new grid code: tick 0 is `segments[0].start_ms`, so the first section's beats follow
its start; `segments[1].start_ms` is unchanged; because `segments[1]` starts on a beat (detected, placed, or
made placed by the move), `bases()` rounds its base to the nearest whole beat counted from the new first
beat and stretches the first section's last beat to reach it (1/2..3/2 beats; the existing rule). Sections
2+ keep their ms positions and their own grids (their bases shift by whole beats only). Loops keep their
time through `apply_tempo_change` (re-snapped ticks), as for every tempo edit.

`player_state.rs`: `SCHEMA = 7`; reads 1-7, writes 7. `found_ms` is read only on the segment that ends up
first after sorting, only as a finite number `0 ≤ v ≤ MAX_SEGMENT_MS`; on any other segment it is dropped.
Why a bump: a schema-6 build parses segments into typed structs and would drop `found_ms` on its next
write, silently turning a set first beat back into an unset one (the intro would play and export again).
As with every earlier bump, schema-6 builds now see the file as newer and never overwrite it. Files of
schema ≤ 6 have no `found_ms`: they play and export as before (requirement 6).

### 2.2 Finding the start (`src/first_beat.rs`, new, pure)

```rust
pub const LEVEL_DB: f64 = -30.0;      // threshold below the loudest 50 ms (power ratio 10^(-3))
pub const WINDOW_MS: f64 = 10.0;      // envelope window
pub const LOUD_WINDOWS: usize = 5;    // the "loudest part" = the loudest 50 ms (5 windows)
pub const SUSTAIN: usize = 3;         // at least 3 of the 5 windows from the onset are above the threshold
pub const REFINE_MS: f64 = 1.0;       // onset refinement window
pub const REFINE_DB: f64 = -6.0;      // refinement level below the threshold
pub fn find_start(pcm: &Pcm) -> f64;  // ms from the file start; 0.0 when nothing rises
pub fn analysis_source<'a>(sources: &'a [Source], loaded: &str) -> Option<&'a Source>;
```

Algorithm (exact, so tests and implementation agree):
1. `w = round(rate · 0.010)` frames. Window power `p[k]` = mean over the window's frames of the mean over
   channels of `(s / 32768)²` (the last, partial window uses its own length).
2. `loud = max_k mean(p[k .. k+5])` (fewer windows at the end: mean of those present). `loud == 0` → `0.0`.
3. `thr = loud · 10^(LEVEL_DB/10)`.
4. The onset window is the first `k` with `p[k] ≥ thr` and at least `min(SUSTAIN, n − k)` of
   `p[k .. k+5]` `≥ thr` (an isolated click in the lead-in noise is not "music"). None → `0.0`.
5. Refinement: in 1 ms sub-windows (`round(rate · 0.001)` frames) from frame `max(0, (k−1)·w)` to
   `(k+1)·w`, the first one with power `≥ thr · 10^(REFINE_DB/10)` gives frame `f` (else `f = k·w`).
6. Result `f · 1000 / rate` ms (not snapped to any grid).

Why our own detector: the spec asks for "rises clearly above the leading silence or noise, judged relative
to the track's loudest part", which is a level test, not beat tracking; it is a few dozen lines over the
16-bit PCM the Player already decodes (claxon for FLAC, the external ffmpeg otherwise; no symphonia, no new
dependency, no server round trip, runs in well under a second for 15 minutes). Alternatives: the edge-AI
beat tracker's first beat (needs the server and a stem render, and its first beat is not where the sound
starts), a spectral-flux onset detector (better for picking beats inside music, unnecessary for the first
rise out of silence/noise), `calliope_lib::flac_level` (100 ms windows, absolute -40 dBFS: too coarse and
not relative). The level is a constant, not a setting; the hand adjustment covers the rest.

**Which audio.** `analysis_source`: the first `backing:*` source, else `audio`, else `original`, else the
source the Player has loaded. That is the track's backing track (the same file the export uses), whatever
the dropdown shows: all sources of a track are sample-aligned, and a single stem (e.g. vocals) would start
late. When the Player has that source loaded, its in-memory PCM is used; otherwise it is decoded with
`player_source::decode` (FLAC: claxon; others: ffmpeg) off the manager lock and dropped afterwards.

### 2.3 The Player: automatic run, Find start, the playback floor (`player.rs`, `player_ipc.rs`)

**Automatic run, once** (requirement 1.1). In `player_ipc::do_open_player`, after the off-lock
`player::load` and before `finish_open`: if `loaded` wants a first beat (`state.tempo.needs_first_beat()`
and the state is not read-only), the detector runs off-lock on the analysis source (the loaded PCM when it is
that source). Then, under the lock, `finish_open` followed by
`PlayerManager::place_first_beat(id, found_ms, FirstBeatVia::Auto, now)`, which re-checks
`needs_first_beat()` on the session's state (it may have been re-read by `finish_open`) and otherwise logs
`skipped`. "Once" is remembered by `found_ms` itself: the placement stores it (even when the music starts at
0:00 and nothing moves), and the debounced write persists it. A decode failure of a separate analysis
source skips the run (logged, retried at the next open); the open itself never fails because of it.
`PlayerManager::open` (the whole-call form used by `player_tests` and `acceptance_player_backend`) does
not run the detector, so the existing manager-level tests keep their maps. Why in the IPC open and not in
`finish_open`: the detector (and a possible second decode) must run without the manager lock, like the
decode itself.

**Find start** (requirement 1.2): a new `player_tempo` action `{"kind": "find-start"}` (no new command, so
no ACL change). `do_player_tempo` → `host.with(m.first_beat_input(id))` (the analysis source's key and
absolute file, its loaded `Arc<Pcm>` when loaded, the ffmpeg path; `Err("Type a BPM first")` on an empty
map, `Err("The source is still loading")` while switching) → decode if needed + `find_start` off-lock →
`host.with(m.place_first_beat(id, ms, FirstBeatVia::Button, now))`. It moves the first beat even when it is
not at 0:00 (owner). The command runs in `spawn_blocking` already.

**Moving the first bar** in the Player: `SectionEdit::Move { index: 0, .. }` (the existing
`move-change` action with index 0, stale guard included) calls `move_first_beat(steps, end_ms)` instead of
`move_change`; the log line is the existing `player section move ... index=0 ... steps= applied=`.

`place_first_beat` and the move both go through the existing live path: `apply_tempo_change` → `sync_grid`
→ `sync_loop` → debounced write, then the floor clamp below. Playback is never interrupted.

**The playback floor** (requirement 3). `Session::floor() = frame_of_ms_f(tempo.play_start_ms())` (nearest
frame: a first beat between two frames rounds so the count-in lead is 0, see below). Every position the
manager sets is clamped to `floor..=frames`: `Session::set_pos` clamps and returns the frame actually set,
and each caller sends that frame (`Seek`/`Jump`) — `seek`, `nudge`, `skip` (Rew/FF), `jump_section` (Start of
section 1 = the first beat via `section_jump_ms`), loop Prev/Next and Stop, loop Play's start, the restart
at the track end (`play_with` and `tick`'s "ended" path: back to the floor instead of 0), and `finish_open`'s
restored position. After every tempo change (`set_bpm`, `edit_sections`, `place_first_beat`) a position below
the new floor is moved to it (`Jump` while playing, `Seek` otherwise). `loop_spec()` clamps the loop start to
the floor (a loop whose end is at or before it is not played; the stored loop is unchanged). The engine gets
the floor with a new `Cmd::SetStart(u64)` (sent with the grid in `sync_grid`) and uses it in its own
`Play`/`PlayLoop` fallback "at the end, restart" instead of 0. Why in the manager: all positions already
come from it, so one clamp covers the UI's slider, time box, keys and buttons; the frontend needs no rule of
its own (it only dims).

**Count-in at the first beat**: nothing new. Play at the floor with the first section's Count In plays the
six ticks over 5 beats and the audio from the floor at the 6th beat position, because the floor is on the
first beat (`beat_lead` = 0; the nearest-frame floor keeps it within half a frame). Requirement 3's "after the
count-in when the first section has Count In on" is the existing pre-roll.

Logs (Rust, `calliope: `): `player first-beat id=<id> via=<auto|button> source=<key> found_ms=<ms, 1 decimal>
start_ms=<ms, 3 decimals> clamped=<bool>`; `player first-beat skipped id=<id> reason=<set|not-at-zero|read-only|decode>`
(only at open, only for a track with a tempo).

### 2.4 The Editor (`ipc.rs`, `editor_tempo`)

`editor_tempo` `move {index: 0, steps}` (absolute steps from `found_ms`, else from the current start) goes
through the existing `TempoStore::replace` path (flush + close the Player session) with
`old.move_first_beat(steps, editor duration_ms)` instead of `set_change_offset` (which keeps refusing index 0
for every other caller). The Editor shows the dimmed region and the movable first bar, but **its own
playback is not restricted** (it is the stem-mix preview; see open question 1). No automatic run and no Find
start button in the Editor (owner: "a Find start button in the Player").

### 2.5 The export (`export_audio.rs`, `export_job.rs`, `export_ipc.rs`)

`CountInPlan` gains `skip_frames: u64`. With `first = round(start_ms · R/1000)` as today:
`skip = first` when `tempo.first_beat_set()`, else `0`; ticks at `(first − skip) + round(k·beat)`;
`first_beat_frame = lead + first − skip`. `run` drops the first `skip_frames` frames of the decoded stream
(counted across chunks) before writing, so the cut is frame-exact at the output rate (also after the RC-5
resampling) and needs no ffmpeg seek. With a set first beat the file is the 5-beat lead (ticks at
0, 1, 2, 2.5, 3, 3.5 beats, the 5th beat silent) followed directly by the audio from the first beat; its
length is `5·beat + frames − skip` (requirement 4). Unset (every older file): `skip = 0`, byte-identical to
today. `export_job`'s progress total becomes `lead + duration_frames − skip`; the start log line gains
`skip_frames=<n>`. `export_info.count_in` gains `skip_ms` (= `play_start_ms()`, 0 when not set) and the
Export pane's count-in line says what is left out.

### 2.6 Frontend

- `src/ui/lib/ipc.ts`: `TempoSegment.found_ms?: number | null`, `TempoAction` `{kind: 'find-start'}`,
  `CountInInfo.skip_ms: number`; the `TempoEdit` comment (steps from `found_ms` for index 0).
- `src/ui/lib/waveform-geometry.ts` (pure, mirrored from Rust): `playStartMs(segs)`, `stepBpm(segs, i)`
  (the BPM whose 1/64 beat moves bar `i`: `segs[0].bpm` for 0, else `segs[i−1].bpm`), `moveRefMs(segs, 0)` =
  `found_ms ?? start_ms`, `moveLimit(segs, 0)` = `Infinity`, `firstBeatLimits(segs, durationMs)` →
  `[0, hi]`, `firstBeatSteps(refMs, pointerMs, bpm, lo, hi)` (the Rust clamp: `nmin = floor`, `nmax = ceil`),
  `firstBeatAt(refMs, steps, bpm, lo, hi)` (= `clamp(ref + steps·fine, lo, hi)`), `sectionMarks` gives index 0 a
  real `x`.
- `WaveformView.svelte` (shared by the Player and the Editor's Mix lane): the first bar is drawn at its start
  (also at x 0) with its label right of it; it is grabbable and keyboard-movable in both modes (Player:
  `freeMove`; Editor: the movable set is index 0 plus the detected changes). The drag preview, the keys
  (Left/Right 1/64 beat, Shift 1 beat) and the readout use `firstBeatSteps`/`firstBeatAt` for index 0; the
  readout reads `+3/64 beat (+0:00.023) from the found start`; the aria value text of section 1 ends with
  `, first beat, +3/64 beat` when set. When the first beat is set and `playStartMs > 0`, the part
  `[0, playStartMs)` is covered after the waveform columns with `--background` at 60 % opacity (marks,
  sections and the playhead are drawn above it).
- Player Sections group, button row: a text button "Find start" after `▶|` (3.25 rem high, aria-label "Find
  where the music starts and put the first beat there"), disabled without a tempo ("Type a BPM first") and
  while running (title "Finding the start..."); the section message line shows `Finding the start...`, then
  `First beat at 0:01.200` (muted), or the error (accent, as other section errors).
  `player-state.svelte.ts`: `findStart()` with a busy flag and generation guard, `pl.sectionNote`; frontend
  logs `player first-beat find` and `player first-beat start=<ms>`. Layout must stay as measured at 1280x800
  (`player layout overflow=0 ... loops_beside=1`).
- Export pane: `countInText` → `Count-in: 5 beats at 120 BPM, the last one silent before the first beat
  (adds 2.5 s; the 1.2 s before the first beat are left out)` when `skip_ms > 0`, unchanged otherwise.

### 2.7 Backwards compatibility summary

- `player.json` ≤ 6 (no `found_ms`): floor 0, export untrimmed, first bar movable (a move sets it), the
  automatic search runs only when the first beat is exactly at 0.0.
- Written by this build: schema 7; schema-6 builds leave it alone ("written by a newer version").
- `track.json` and the audio files are never touched (out of scope: "the part before the first beat stays in
  the files").

## 3. Tasks

Every task: Rust formatted with `cargo fmt --all`; run only the tests the task touches (named below), the
lead runs `npm test` once per task before committing. `npm` commands run in `src/calliope-gui/`.

### Task 1. Tempo map: the set first beat and its operations
- routine: no
- files: `src/tempo_map.rs`; every `Segment { .. }` literal gets `found_ms: None`: `src/editor_tests.rs`,
  `src/export_audio_tests.rs`, `src/export_job_tests.rs`, `src/metronome.rs`, `src/player_ipc_tests.rs`,
  `src/player_state.rs`, `src/player_tests.rs`, `src/tempo_detect.rs`, `src/tempo_detect_tests.rs`,
  `src/tempo_store.rs`, `tests/acceptance_beat_autodetect.rs`, `tests/acceptance_player_backend.rs`.
- does: the `found_ms` field and the functions of §2.1 table (`first_beat_set`, `play_start_ms`,
  `needs_first_beat`, `first_beat_limits`, `place_first_beat`, `move_first_beat`, `section_jump_ms(0, Start)`,
  `set_bpm_at` removal clearing `found_ms`), with doc comments. Unit tests in `tempo_map.rs`:
  - place at 1200 on a 120 BPM single-section map: `start_ms == 1200`, `found_ms == Some(1200)`; place at
    -5 → 0.0; place past `end − 1 beat` → exactly `end − 500`.
  - move +1 from found 1200 at 120 BPM → `start == 1200 + 7.8125`, applied 1, `found_ms` still 1200; move
    -10_000 → start exactly 0.0, applied = `floor(-1200/7.8125)` = -154; move +10_000 with a second section at
    3000 → start exactly 2500.
  - move on a never-found map (start 800): `found_ms` becomes `Some(800)`, offsets counted from 800.
  - three sections (0 → 120 BPM, 4000 detected 90 BPM, 8000 typed 100 BPM): after moving the first beat by
    +0.3 beat, `segments[1..]` `start_ms` unchanged, `ms_at(0) == new start`, `ms_at(16·k)` = start + k·500 for
    beats inside section 1, `ms_at(bases()[1]) == 4000`, the stretched last beat is within 1/2..3/2 beats,
    `ms_at`/`tick_near` stay monotonic and inverse across 0..10 s; `bases()[2] − bases()[1]` unchanged.
  - a typed second section becomes `on_beat` after a move; a place at the current start changes nothing but
    `found_ms`.
  - `play_start_ms`: 0 when unset (start 1200, no `found_ms`), 1200 when set, 0 on an empty map;
    `needs_first_beat`: true only for start exactly 0.0 without `found_ms`.
  - `section_jump_ms(0, Start)` = 1200 when set, 0 when unset; `set_bpm_at(pos in section 1, 0)` on a
    two-section map leaves a first segment without `found_ms`.
  - serde: `found_ms` round-trips on a segment and is omitted when `None`.
- done when: `cargo test -p calliope-gui --lib tempo_map` passes with the tests above;
  `cargo build --workspace --all-targets` succeeds; `cargo fmt --all --check` is clean.

### Task 2. `player.json` schema 7
- routine: no
- files: `src/player_state.rs`; schema assertions `6 → 7` and the "newer file" texts `7 → 8` in
  `src/beat_detect_tests.rs`, `src/tempo_store.rs`, `tests/gui_editor_e2e.rs`, `tests/gui_player_e2e.rs`,
  `tests/gui_player_acceptance_e2e.rs`, `tests/acceptance_beat_autodetect.rs`,
  `tests/acceptance_player_backend.rs` (find them with
  `grep -rnE 'schema_version"?\]?:? ?7|"schema_version"\], ?6' src tests`); the module doc of `player_state.rs`.
- does: `SCHEMA = 7`; `parse_tempo` reads `found_ms` (finite, `0 ≤ v ≤ MAX_SEGMENT_MS`) and keeps it only
  on the segment that is first after sort/dedup. Fixture files that are schema 6 on purpose (the export and
  player fixtures) stay as they are: they are the "saved before this feature" cases.
  Tests in `player_state.rs`: a v7 file with `found_ms` on the first segment round-trips; `found_ms` on a
  second segment, negative, `NaN`-like strings or `true` is dropped; a v6 file reads with `found_ms: None` and
  is written as 7; a v8 file is read-only (the existing newer test, renumbered).
- done when: `cargo test -p calliope-gui --lib player_state tempo_store beat_detect` passes; `cargo test
  --workspace --no-run` and `cargo test --features e2e-hooks --no-run` build; the grep above finds no
  `"schema_version"], 6` assertion left.

### Task 3. The start detector
- routine: no
- files: `src/first_beat.rs` (new), `src/lib.rs` (`pub mod first_beat;`).
- does: §2.2 exactly (`find_start`, `analysis_source`, the constants). Unit tests with synthetic PCM built in
  the test (deterministic LCG noise, no files):
  - 1.2 s of white noise at about -50 dBFS RMS, then 3 s of a 440 Hz sine at amplitude 0.5, stereo 44.1 kHz:
    within 3 ms of 1200 (the acceptance bound is 10 ms).
  - the same at 48 kHz and mono: within 3 ms.
  - music from frame 0 (sine from 0): exactly 0.0; all digital silence: 0.0; noise only: 0.0 (the loudest
    part is the noise itself, so the first window qualifies).
  - a single 2 ms full-scale click at 0.5 s inside the noise lead is ignored (result still ≈ 1200).
  - a quiet intro at -20 dB relative to the loudest part starting at 0.8 s, loud part at 2.0 s: ≈ 800.
  - a sine fading in linearly over 1 s from 1.0 s: the crossing of -30 dB relative (≈ 1.0316 s ± 15 ms).
  - `analysis_source`: backing variant before `audio` before `original`; a stems-only track falls back to the
    loaded key; returns `None` for an empty list.
- done when: `cargo test -p calliope-gui --lib first_beat` passes; clippy clean for the module
  (`cargo clippy -p calliope-gui --all-targets -- -D warnings`).

### Task 4. Noisy-intro test fixture
- routine: yes
- files: `tests/fixtures/first-beat/make-fixtures.sh` (new, modelled on `tests/fixtures/export/make-fixtures.sh`:
  `set -euo pipefail`, the same `ff` helper with `-fflags +bitexact`, idempotent unless `FORCE=1`),
  `tests/fixtures/first-beat/noisy-intro.flac` (generated and committed).
- does: one file, 44.1 kHz stereo s16 FLAC, 5.2 s: 1.2 s of white noise
  (`anoisesrc=color=white:amplitude=0.005:seed=1:sample_rate=44100`, mono duplicated to stereo), then 4.0 s of
  a 440 Hz sine at amplitude 0.5 (`sine=frequency=440:sample_rate=44100`, `volume=4` since the sine filter
  peaks at 1/8), concatenated with the `concat` filter exactly as the 407 "Long Intro" fixture does. No
  `track.json`: tests copy the file into their temp repositories.
- done when: `bash tests/fixtures/first-beat/make-fixtures.sh` exits 0 twice in a row (second run keeps the
  file); `ffprobe -v error -show_entries stream=sample_rate,channels:format=duration -of csv=p=0
  tests/fixtures/first-beat/noisy-intro.flac` prints rate 44100, 2 channels and a duration of 5.2 s (± 0.01);
  the file is under 1.5 MB.

### Task 5. Player engine and manager: floor, first-beat placement, moving bar 1
- routine: no
- files: `src/player.rs`, `src/player_tests.rs`.
- does: §2.3 manager/engine part: `Session::floor`, `set_pos` clamping and returning the frame, every
  position path listed in §2.3, the clamp after tempo changes, `loop_spec` start clamp, `Cmd::SetStart` (sent
  from `sync_grid`, used in the engine's restart fallback), `SectionEdit::Move { index: 0 }` →
  `move_first_beat`, `FirstBeatVia { Auto, Button }`, `FirstBeatInput` + `first_beat_input(id)`,
  `place_first_beat(id, ms, via, now)` (Auto re-checks `needs_first_beat()`; logs of §2.3),
  and `Loaded` accessors needed by Task 6 (`wants_first_beat()`, `source_key()`, `pcm()`). Tests in
  `player_tests.rs` (ManualBackend, temp copies only), with `player.json` written into the copy
  (`found_ms: 1200, start_ms: 1200, 120 BPM, count_in: true`):
  - open with a saved position 0 → position 1200; `seek(0)`, `nudge(-5000)`, `skip(-5000)`, section 1 Start,
    loop Prev to a loop starting at 500, loop Stop → all land on frame `round(1.2·rate)`.
  - Play from the floor with Count In: six clicks at output frames `round(k·beat)`, k = 0, 1, 2, 2.5, 3, 3.5,
    and the first source frame heard (= the floor frame) at output frame `5·beat` exactly; a first beat at
    1203.47 ms gives the same (lead 0).
  - play to the end, Play again → restarts at the floor.
  - the same file without `found_ms` (schema 6): position 0 allowed, Play from 0 plays frame 0 (as before).
  - `edit_sections(Move { index: 0, at, steps: 1 })` → start + 1/64 beat, other sections' `start_ms`
    unchanged, `found_ms` unchanged, `player.json` written after the debounce; steps beyond the limits clamp
    to 0.0 / next − 1 beat; stale `at` refused.
  - `place_first_beat(.., Auto, ..)` on a map whose start is 500 → unchanged + `skipped reason=not-at-zero`;
    on start 0.0 without `found_ms` → placed; Button on start 500 → placed.
  - a move of the first beat later than the paused playhead moves the playhead to the new floor.
- done when: `cargo test -p calliope-gui --lib player_tests` passes, and the existing
  `cargo test -p calliope-gui --test acceptance_player_backend --test player_realtime` still pass.

### Task 6. Player IPC: automatic run at open and the `find-start` action
- routine: no
- files: `src/player_ipc.rs`, `src/player_ipc_tests.rs`.
- does: §2.3 "Automatic run" and "Find start" (`TempoAction::FindStart` = `{"kind":"find-start"}`).
  Tests in `player_ipc_tests.rs` on the fixture copy, with a plain backing track seeded into the temp
  repository by a small local helper (copies `tests/fixtures/first-beat/noisy-intro.flac` as `audio`, writes a
  minimal schema-2 `track.json` and the given `player.json`):
  - tempo `{start 0, 120}` without `found_ms`: after `do_open_player`, the snapshot's first segment has
    `start_ms` within 10 ms of 1200 and `found_ms == start_ms`; `player.json` holds it after the debounce
    flush (close); a second open does not run again (start moved by hand to 1300 stays 1300; log `skipped
    reason=set`).
  - start 500 without `found_ms` (an owner-set first beat): not moved, `skipped reason=not-at-zero`; then
    `find-start` moves it to ≈ 1200.
  - after `move-change` index 0 by +64 steps, `find-start` brings it back to ≈ 1200 and the offset is 0.
  - no tempo: nothing runs, `find-start` → `Err("Type a BPM first")`.
  - the PRACTICE fixture (music from 0) with a typed tempo at 0: placed at exactly 0.0, `found_ms: 0.0`, no
    position change.
  - the loaded source is a stem while a backing exists: the backing is analysed (`source=backing:...` in the
    log line, read from a captured sink or the returned value).
  - a newer (schema 8) `player.json`: nothing runs.
- done when: `cargo test -p calliope-gui --lib player_ipc` passes.

### Task 7. Editor: move the first bar
- routine: no
- files: `src/ipc.rs` (`do_editor_tempo`), `src/editor_ipc_tests.rs`.
- does: §2.4: `TempoEdit::Move { index: 0, steps }` → `store.replace(.., |old| old.move_first_beat(steps,
  duration_ms))` with the Editor session's duration; other indexes unchanged. Tests: a stem track copy with
  `player.json` (start 500, `found_ms` 500, a detected change at 4000): move index 0 by +1 → `start_ms` 500 +
  1/64 beat, the change at 4000 unchanged, schema 7, a Player session on the same track is closed (existing
  release behaviour); move by -10_000 → 0.0; during a running detection → refused as today.
- done when: `cargo test -p calliope-gui --lib editor_ipc` passes.

### Task 8. Export without the audio before the first beat
- routine: no
- files: `src/export_audio.rs`, `src/export_audio_tests.rs`, `src/export_job.rs`, `src/export_job_tests.rs`,
  `src/export_ipc.rs`, `src/export_ipc_tests.rs`.
- does: §2.5. Tests:
  - `count_in_plan`: set first beat 1200 at 120 BPM, 44.1 kHz: `skip_frames == 52920`, ticks
    `[0, 22050, 44100, 55125, 66150, 77175]`, `first_beat_frame == lead == 110250`; unset: `skip_frames == 0`
    and the existing values (existing tests keep passing).
  - `run` with the real local ffmpeg on `tests/fixtures/first-beat/noisy-intro.flac` copied to a temp dir and
    a set first beat 1200 (WAV output): length `110250 + 229320 − 52920`, frames `[0, lead)` zero except
    under the six ticks, frames `lead..` equal the source from frame 52920 on (bit-exact), no sample of the
    noise lead in the output.
  - `export_info`: `count_in.skip_ms` 1200 when set, 0 when unset.
  - progress reaches 1.0 on a trimmed export (job test).
- done when: `cargo test -p calliope-gui --lib export_audio export_job export_ipc` passes and
  `cargo test -p calliope-gui --test acceptance_export` still passes unchanged (older files export as before).

### Task 9. Waveform geometry helpers for the first bar
- routine: yes
- files: `src/ui/lib/waveform-geometry.ts`, `src/ui/lib/waveform-geometry.test.ts`.
- does: add, with JSDoc, exactly these (segments typed as the file's `MoveSeg` plus `found_ms?: number | null`):
  - `playStartMs(segs)`: `segs[0].found_ms != null ? Math.max(0, segs[0].start_ms) : 0`; 0 for `[]`.
  - `stepBpm(segs, i)`: `i === 0 ? segs[0].bpm : segs[i - 1].bpm`.
  - `moveRefMs(segs, i)`: for `i === 0` return `segs[0].found_ms ?? segs[0].start_ms`; others unchanged.
  - `moveLimit(segs, i)`: `Infinity` for `i === 0`; others unchanged.
  - `firstBeatLimits(segs, durationMs)`: `[0, hi]`, `hi = (segs[1]?.start_ms ?? durationMs) - 60000 / segs[0].bpm`.
  - `firstBeatSteps(refMs, pointerMs, bpm, lo, hi)`: `fine = fineMs(bpm)`; `nmin = Math.floor((lo - refMs) / fine)`;
    `nmax = Number.isFinite(hi) ? Math.ceil((hi - refMs) / fine) : Infinity`; returns
    `clamp(roundHalfAway((pointerMs - refMs) / fine), nmin, nmax)`.
  - `firstBeatAt(refMs, steps, bpm, lo, hi)`: `Math.min(Math.max(refMs + steps * fineMs(bpm), lo), hi)`.
  - `sectionMarks`: index 0 gets `x: xOf(s.start_ms, durationMs, width)` (the `SectionMark.x` doc comment
    updated: never null now; keep the `number | null` type).
  Tests: `playStartMs` set/unset/empty; `firstBeatSteps(1200, 1207.9, 120, 0, 2500) === 1`;
  `firstBeatSteps(1200, -50, 120, 0, 2500) === -154` and `firstBeatAt(1200, -154, 120, 0, 2500) === 0`;
  `firstBeatSteps(1200, 9999, 120, 0, 2500) === 167` and `firstBeatAt(..., 167, ...) === 2500`;
  `firstBeatLimits` with and without a second section; `moveRefMs` for 0 with and without `found_ms`;
  `sectionMarks` gives index 0 its x (0 at start 0).
- done when: `npx vitest run src/ui/lib/waveform-geometry.test.ts` passes and `npm run check` is clean.

### Task 10. Waveform view: the first bar, the readout and the dimmed lead-in
- routine: no
- files: `src/ui/lib/ipc.ts` (`TempoSegment.found_ms`, `TempoAction` `find-start`, `CountInInfo.skip_ms`,
  the `TempoEdit` comment), `src/ui/components/player/WaveformView.svelte`,
  `src/ui/components/player/WaveformView.test.ts`, `src/ui/components/editor/EditorPane.test.ts` (if its
  expectations about index 0 change), `src/ui/lib/fixture-tracks.ts` (only if a fixture needs `found_ms`).
- does: §2.6 WaveformView part, both modes. Replace every `segments[i - 1].bpm` used for moving with
  `stepBpm`, the index-0 limits with `firstBeatLimits`, the steps/preview with `firstBeatSteps`/`firstBeatAt`;
  remove the `sel === 0` early return for Left/Right; the Editor's movable set = index 0 (when a tempo exists)
  + detected changes; readout for index 0 `±N/64 beat (±m:ss.mmm) from the found start` (no limit tail); the
  dim overlay. Tests (canvas spied as today): the first bar is a `fillRect` at x of 1200 ms; pointer down on
  it, move by one fine step's pixels, up → `onmove(0, 1)` and the readout text during the drag contains
  `+1/64 beat` and `from the found start`; Shift+Left ×40 from the found position → the last `onmove(0, n)` has
  the 0:00 step (-154 at 120 BPM, 1200 ms); Right past the limit stops at the `nmax` step; with `found_ms` a
  rect of width `xOf(1200)` and the background colour at 0.6 alpha is drawn, without `found_ms` (start 1200)
  none; Editor mode (no `freeMove`): index 0 is selectable with Up/Down and movable; aria value text of section
  1 contains `first beat, +1/64 beat` when moved.
- done when: `npx vitest run src/ui/components/player/WaveformView.test.ts src/ui/components/editor` passes;
  `npm run check` clean; `npm run test:ui` passes.

### Task 11. Player: the Find start button
- routine: no
- files: `src/ui/lib/player-state.svelte.ts`, `src/ui/components/player/SectionsSection.svelte`,
  `src/ui/lib/player-state.test.ts`, `src/ui/components/player/ControlBar.test.ts`.
- does: §2.6 Player part: `findStart()` (busy flag, generation guard against a track switch, `applySnapshot`,
  `pl.sectionNote = 'First beat at m:ss.mmm'`, errors into `pl.sectionError`, logs), the button and its
  disabled titles, the message line showing `sectionError || sectionNote || NEED_BPM`; any other section
  action clears the note. Tests (mockIPC): the button calls `player_tempo` with `{kind: 'find-start'}`, is
  disabled while the call runs and without a tempo (title "Type a BPM first"), shows the note with the new
  start, shows a refusal in the message line; a reply arriving after a track switch is ignored.
- done when: `npx vitest run src/ui/lib/player-state.test.ts src/ui/components/player` passes;
  `npm run check` clean.

### Task 12. Export pane: what the count-in line says
- routine: yes
- files: `src/ui/lib/export-format.ts`, `src/ui/lib/export-format.test.ts`.
- does: in `countInText`, when `c.skip_ms > 0`, the parenthesis becomes
  `(adds ${seconds(c.lead_ms)} s; the ${seconds(c.skip_ms)} s before the first beat are left out)`; otherwise
  the text stays exactly as it is now. Tests: `skip_ms: 0` gives the current text; `{bpm: 120, first_beat_ms:
  1200, lead_ms: 2500, skip_ms: 1200}` gives `Count-in: 5 beats at 120 BPM, the last one silent before the
  first beat (adds 2.5 s; the 1.2 s before the first beat are left out)`. Existing test objects get
  `skip_ms: 0`.
- done when: `npx vitest run src/ui/lib/export-format.test.ts src/ui/views/ExportView.test.ts` passes and
  `npm run check` is clean (after Task 10 added `skip_ms` to the type).

### Task 13. Player GUI e2e: first beat end to end
- routine: no
- files: `tests/gui_player_e2e.rs` (new test fn `player_first_beat`; helpers stay in the file).
- does: on `player_dirs` + a seeded noisy-intro plain backing track (copy of the Task 4 file as `audio`,
  `track.json`, `player.json` schema 6 with `{start_ms: 0, bpm: 120, count_in: true}`):
  1. open it in the Player → `calliope: player first-beat ... via=auto ... start_ms=` within 10 ms of 1200;
     `player.json` gets `found_ms` (wait_json).
  2. screenshot `target/gui-shots/player-first-beat.png` (dimmed lead-in, bar at 1.2 s): reviewed by looking
     at it.
  3. Home and Rew → `player seek`/position lines at ≥ 1200 ms (never below).
  4. Play from the start (capture WAV): six click onsets at k·500 ms after the first one (± 3 ms), the first
     440 Hz energy at 2500 ms after the first click (± 10 ms), and no noise energy before it outside the clicks.
  5. focus the waveform, Up to section 1, Right → `calliope: player section move ... index=0 ... steps=1
     applied=1`, and `player.json` (wait_json) has `start_ms` = found + 7.8125 with `found_ms` unchanged; a
     short xdotool drag of the first bar (`drag_xy`-style: mousedown, stepped mousemove, screenshot
     `player-first-beat-drag.png` while the button is held, mouseup) shows the readout "from the found start".
  6. press "Find start" → `player first-beat ... via=button` with start ≈ 1200 again.
  7. `player layout overflow=0 ... loops_beside=1` at 1280x800 with the tree folded (the button fits).
- done when: `npm run test:gui -- player_first_beat` passes (or `DISPLAY=:1 npm run test:gui:serial --
  player_first_beat`) and the two screenshots were looked at: the region before 1.2 s is visibly dimmed, the
  first bar and its label are readable, the readout is legible in both themes (dark default; one light
  screenshot via Settings is enough).

### Task 14. Editor GUI e2e: first bar in the Mix lane
- routine: no
- files: `tests/gui_editor_e2e.rs` (new test fn `editor_first_beat`).
- does: Editor fixture copy with a `player.json` written at test time (schema 7, `start_ms 500, found_ms 500,
  120 BPM` plus one detected change); open the Editor, focus the Mix waveform (the first selectable bar is
  section 1), Right → `editor tempo move index=0 steps=1 applied=1` and `player.json` `start_ms` = 500 +
  7.8125 (wait_segments); Shift+Left ×20 → `start_ms` 0.0; screenshot `editor-first-beat.png` shows the bar
  and (before the move to 0) the dimmed lead-in.
- done when: `npm run test:gui -- editor_first_beat` passes and the screenshot was looked at.

### Task 15. Docs: README and UI guide
- routine: yes
- files: `src/calliope-gui/README.md`, `docs/ui.md` ("Decided by the team" only).
- does: README Player usage: one paragraph "First beat": found automatically on the first open when the first
  beat is at 0:00, "Find start" button, drag/keys on the first bar (1/64 beat, Shift = 1 beat, 0:00 up to one
  beat before the next section), the audio before it is dimmed, not played and not exported; older tracks
  unchanged until found or moved. `docs/ui.md`: append one bullet `(gui-first-beat)` describing the dimmed
  lead-in (background at 60 % over it), the first bar (drawn also at 0:00, label right of it, same drag/keys as
  other bars, readout "+N/64 beat (time) from the found start"), and the "Find start" button after `▶|` with
  its message line texts ("Finding the start...", "First beat at 0:01.200").
- done when: `git diff --stat` shows only those two files; `npm test` still passes (no code changed).

## 4. Test strategy

- **Pure logic** (unit, `cargo test --lib`): the tempo map's first-beat rules and grid consequences
  (`tempo_map.rs`), the schema-7 read/write (`player_state.rs`), the detector on synthetic signals
  (`first_beat.rs`), the export plan and frame-exact cut (`export_audio_tests.rs`, real local ffmpeg on the
  committed fixture), TS geometry mirrors (`waveform-geometry.test.ts`).
- **Engine and manager** (`player_tests.rs`, and the tester's `tests/acceptance_player_backend.rs` additions):
  `ManualBackend` only, exact click/source frames for the count-in at the first beat, every clamp path, loops
  starting before the floor, legacy files.
- **IPC level** (`player_ipc_tests.rs`, `editor_ipc_tests.rs`, `export_ipc_tests.rs`): the automatic run at
  open, once, its skip reasons, Find start, the analysis source rule, the Editor's move through the
  `TempoStore` path. Temp copies of the committed fixture repositories plus the noisy-intro file, never the
  user's repository.
- **Export acceptance** (tester, `tests/acceptance_export.rs`, `ac_first_beat_*`): the noisy-intro track
  added to the `library-export` temp copy with a set first beat: RC-5 WAV length/ticks/bit-exact audio from
  1.2 s, MP3/OGG duration ± 30 ms, `repo_unchanged_except_exports`; the existing count-in tests (schema-6
  files) stay unchanged and keep passing = "saved before this feature exports as before".
- **Frontend** (vitest + jsdom, mockIPC): WaveformView drag/keys/readout/dim (canvas spied), the Find start
  flow, the Export count-in text.
- **GUI on a display**: `gui_player_e2e::player_first_beat`, `gui_editor_e2e::editor_first_beat`; capture
  WAV analysis for the count-in; screenshots in `target/gui-shots/` viewed by the agent; the layout log for
  the new button. Waits use the existing knobs (log lines, `wait_json`, `wait_segments`); no sleeps beyond the
  existing short helpers.
- **No hardware, no network**: audio output is `ManualBackend`/`NullBackend` capture only; the detector is
  local; no test needs the edge-AI server.
- Whole headless suite: `cd src/calliope-gui && npm test`. GUI suite: `cd src/calliope-gui && npm run test:gui`
  (or `DISPLAY=:1 npm run test:gui:serial`).

## 5. Deployment

Single machine: everything changed runs in calliope-gui on the laptop (`npm run build:app`). The
`calliope-stems` server is not touched.

## 6. Manual checks (owner, on the laptop)

1. Open a real song with silence, noise or a spoken count before the music, that already has a tempo whose
   first beat is at 0:00, in the Player. The first bar should jump to where the music starts, and the part
   before it should be dimmed.
2. Turn on the first section's Count In, press Home and then Play. You should hear six ticks, a silent beat,
   and the music's first beat landing right on the next count. Listen for whether it feels on time.
3. Drag the first bar a little, or focus the waveform, select section 1 and use Left/Right and Shift+Left/Right.
   The readout should show the offset from the found start. Play it again and check the count-in follows.
4. Press "Find start". The bar should go back to the found position.
5. On a track with several sections, move the first beat. The later sections' bars should stay where they are,
   and the metronome (Click on) should still sit on the beats of every section.
6. Export that track to the RC-5 and to MP3. Play the file: it should start with the count-in leading straight
   into the music, without the noise or silence that came before.
7. Open an older track whose first beat you placed yourself (not at 0:00). Nothing should move, Play from the
   start should still play the intro, and its export should sound like before. Then press "Find start" on it
   once to see the new behaviour.
8. In the Editor, the same track should show the dimmed lead-in, and the first bar should move with the keys.
   Editor playback still plays from 0:00 (see open question 1).

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests |
|---|---|---|
| 1.2 s of low noise → first section within 10 ms of 1.2 s | 3, 4, 6, 13 | `first_beat` noise test (± 3 ms); `player_ipc_tests` automatic run on noisy-intro; `gui_player_e2e::player_first_beat` step 1 |
| Drag 1/64 beat later → moves, readout shows the offset from the found position | 1, 5, 9, 10, 13, 14 | `tempo_map` move +1; `player_tests` Move index 0; `WaveformView.test.ts` drag + readout; GUI steps 5 (Player) and `editor_first_beat` |
| First beat at 1.2 s, Play from the beginning with Count In → six ticks, silent beat, music at 1.2 s on the 6th beat position | 5, 13 | `player_tests` count-in frames at the floor (also fractional first beat); tester `acceptance_player_backend` `fb_*`; GUI step 4 (capture WAV) |
| Playhead can't be moved before 1.2 s; region dimmed | 1, 5, 10, 13 | `player_tests` clamp paths; `WaveformView.test.ts` dim rect; GUI steps 2-3 |
| Export = 5-beat lead + audio from 1.2 s, 5th lead beat silent ending at the first beat, no noise | 8, 12 | `export_audio_tests` plan + run on noisy-intro; tester `acceptance_export::ac_first_beat_*`; `export-format.test.ts` |
| No found first beat → placed on first Player open; moved by hand + Find start → back to the found start | 5, 6, 11, 13 | `player_ipc_tests` auto/once/find-start; `player-state.test.ts`; GUI steps 1, 6 |
| Several sections: others unchanged, first section's beats follow | 1, 5 | `tempo_map` three-section test; `player_tests` Move index 0 (other starts unchanged) |
| Dragged past 0:00 / past the next section → stops at 0:00 / just before | 1, 9, 10, 14 | `tempo_map` limits; `waveform-geometry.test.ts`; `WaveformView.test.ts` Shift+Left; `editor_first_beat` Shift+Left to 0.0 |
| Track saved before → plays and exports as before | 2, 5, 6, 8 | `player_state` v6 read; `player_tests` legacy floor 0; `player_ipc_tests` start 500 skipped; existing `acceptance_export` count-in tests unchanged |

## 8. Open questions

Answered by the owner (2026-10-25): all three defaults accepted (Editor plays from 0:00 and only dims;
Autodetect replaces the map as today; the first beat stops one beat before the next section).

1. **Editor playback.** Requirement 3 says the audio before the first beat "is left out of playback". The plan
   applies that to the Player (where the count-in is) and only dims the region in the Editor; Editor Play still
   plays the stem mix from 0:00. Should the Editor's playback also start at the first beat? (Default in the
   plan: no.)
2. **Autodetect BPM after the first beat was set.** A new detection replaces the whole tempo map today, so a
   found or hand-placed first beat is replaced by the detected first downbeat, the region before it is no longer
   left out, and the automatic search won't run again (the first beat is no longer at 0.0). Should Autodetect
   keep the first beat (its start and `found_ms`) instead? (Default in the plan: replace, as today; press "Find
   start" again.)
3. **Upper limit.** "Just before the next section" is planned as one beat (of the first section's tempo) before
   the next section, the rule every other section already follows. Is that close enough, or should the first
   section be allowed to shrink below one beat? (Default: one beat.)
