# Plan: Player speed and speed training

Revision 1. Base `e49615e`, branch `spec/gui-player-speed`. Builds on:
- `specs/gui-play-backing-track.plan.md`: the Player engine (2.7), `player.json` (2.5), the never-built increment B
  (2.12, tasks 19-22), which this plan replaces;
- `specs/gui-player-tempo-sections.plan.md`: per-section Click / Count In and the click list (10.4), the Play and loop
  Play count-in rules (18.4), the two-row control bar (18.2), `player.json` schema 5 (18.3).

The design follows the owner's answers of section 8 (2026-10-18): Q1 the alternative (training settings in a popover),
Q2-Q10 as recommended.

**Revision 2 (2026-10-18, owner answers).** The training settings moved from a fourth line of the Loops pane into a
popover opened from a "Training…" button on the Loops pane's third line (2.9, 2.11, Tasks 14-15, layout budget, GUI
positions). Everything else is unchanged.

## 1. Summary

The Player gets a speed control in the Playback pane: 25 % to 200 %, `-`/`+` in 5 % steps, a box to type a whole
number and Reset, plus a read-only readout of the BPM being played. The pitch is kept by a real-time time-stretcher
(Signalsmith Stretch, MIT, through the `signalsmith-stretch` crate) that runs inside the existing playback callback.
Positions, loops and sections stay in song time; the metronome clicks are still synthesised after the stretch, so
they stay on the music's beats and are not stretched themselves. Each loop gets speed-training settings (start %,
end %, step %, repeats); its Play then speeds up by itself pass after pass. The speed and the training settings are
saved in `player.json`, which becomes schema 6.

## 2. Design

### 2.1 What is reused, what is new

Reused unchanged:
- the engine's command channel, trash channel and atomics (`Cmd`, `Shared`), the 3 ms fades, the count-in machinery
  (`CountIn`, output-frame counting), the click list (`section_clicks`) and the click sound;
- song-time positions everywhere (transport events, waveform, loop boundaries, `position_ms` in `player.json`);
- the `player_tempo` and `player_loop` tagged actions (no new command, no ACL change);
- the speed-1.0 path of the engine: at 100 % it is the exact code of today (every bit-exact test keeps passing).

New:
- `src/speed.rs` (pure): percent rules (range, `-`/`+` steps), the training settings and their pass schedule;
- `src/stretch.rs`: a thin wrapper around `signalsmith_stretch::Stretch` with the alignment rule of 2.4;
- a **stretch mode** in `Engine::render` (2.4), `Cmd::SetSpeed`, and the training run (2.6);
- `player.json` schema 6 (2.7); new transport and snapshot fields (2.8);
- `SpeedControl.svelte` in the Playback pane, a "Training…" button with a settings popover in the Loops pane (2.9,
  2.11), and the shadcn-svelte `popover` component (bits-ui `Popover`, MIT, already a dependency).

### 2.2 Time-stretch library (constraint: permissive licence, real time, laptop, no symphonia)

| Option | Licence | Quality on guitar / full mixes | Real-time cost, latency | Verdict |
|---|---|---|---|---|
| **Signalsmith Stretch 1.3.1** via `signalsmith-stretch` 0.1.3 (Rust FFI, vendored C++) | MIT (binding: Colin Marc; C++: Signalsmith Audio / Geraint Luff; `signalsmith-linear`: MIT) | Phase-vocoder family with tonal/transient handling; widely used in DAWs and plug-ins; the best permissive option known | **Measured on this laptop** (release, stereo 44.1 kHz, 256-frame blocks): 0.8 % of one core at 25-200 %; latency 2646 + 2646 frames (60 + 60 ms); 440 Hz stays 440.0 Hz at 50/85/150/200 % (440.9 Hz at 25 %) | **chosen** |
| `ssstretch` 0.1.0 | MIT | same C++ library, other binding | same | no advantage, younger |
| `timestretch` 0.15 (pure Rust) | MIT | "optimised for EDM", one author, unproven on guitar | unknown | rejected: risk |
| `wsola` 0.2 (pure Rust) / own WSOLA | MIT / ours | WSOLA: fine for speech and single lines, audible doubling and phasing on strummed chords and full mixes | low | rejected: quality |
| Own phase vocoder (rustfft) | ours | reaching Signalsmith's quality (phase locking, transients, stereo coherence) is weeks of DSP tuning | low | rejected: effort, risk |
| Bungee (`bungee-rs`) | MPL-2.0 | good, built for streaming | low | rejected: not permissive (spec), needs CMake |
| SoundTouch, Rubber Band | LGPL-2.1, GPL | | | excluded by the spec |

Consequences of the choice:
- **Build prerequisites:** `clang` (libclang, for bindgen) and a C++ compiler. Both are installed (clang 23, g++); the
  crate built and ran here with them. Add `clang` to the README prerequisites.
- **Debug builds:** the C++ is compiled by the crate's build script with the package's opt-level, so the workspace gets
  `[profile.dev.package.signalsmith-stretch] opt-level = 3` (like `claxon`/`flacenc`), or tests would be slow.
- **Licences:** the crate tree adds `signalsmith-stretch` (MIT) and the `dasp` crates (MIT OR Apache-2.0) to the shipped
  binary; `bindgen`, `cc`, `clang-sys` and friends are build-time only. The two vendored C++ licence files live in
  sub-folders of the crate, so the notices generator must pick them up explicitly (Task 1).
- **Risk:** a young binding with one maintainer. It is about 200 lines over a vendored header library; everything goes
  through `stretch.rs`, so switching the binding (or vendoring it) touches one file.
- **Real-time safety:** the C++ allocates in `configure` and grows one scratch vector on the first `seek`; `Stretcher::new`
  calls `seek` once so the audio thread never allocates (2.4). A Rust counting allocator checks our side (Task 5); C++
  allocations are covered by code reading (they go through `malloc`, which the Rust allocator does not see).

### 2.3 Speed model (requirements 1-4, 7)

- The **track speed** is a whole percentage 25..=200, default 100, one per track (all sources), independent of the
  tempo map. It works without a tempo map (requirement 6, last sentence), which drops the old rule "speed only in BPM
  mode".
- `-`/`+` move to the next multiple of 5 in that direction, clamped (73 → 70 / 75; 100 → 95 / 105; 25 stays 25 with `-`,
  200 stays 200 with `+`). Typing takes a whole number 25..=200 (an optional `%` and spaces are accepted); anything
  else is refused and the speed is unchanged. Reset sets 100.
- Typing a BPM and Autodetect no longer reset the speed (they did under the old BPM-step design, because the speed was
  derived from a BPM). `PlayerManager::set_bpm` and `TempoStore::replace` keep `speed`.
- Positions, the waveform, section starts and loop boundaries stay in song time. A loop 0:10..0:20 is still shown as
  that at 50 %; it takes 20 s to play.
- The readout (requirement 6) is `played_bpm = bpm_at(playhead) × speed / 100`, empty when there is no tempo map. The
  Sections pane's BPM box keeps showing and editing the song BPM (`transport.bpm` loses its `× speed`).
- Keys (owner's `docs/ui.md`): `[` and `]` are `-` and `+`. Held keys do not repeat (as Space, b, e).

### 2.4 Stretch mode in the engine (requirements 2-5, 9, 13)

**Two paths.** The engine plays in *bypass* mode (today's per-frame code, bit-exact) when the speed it plays at is
exactly 100 % and no training run is active, and in *stretch* mode otherwise. The mode is chosen at each
discontinuity (Play from paused, `Jump`, `Resume` after a scrub, `PlayLoop`, a wrap with count-in). Changing the speed
while playing:
- bypass → stretch (e.g. 100 % → 70 %): the stretcher is aligned at the current position (below) and the two paths are
  crossfaded over the 3 ms fade length, so there is no gap and no click;
- stretch → another speed, including 100 %: only the feed ratio changes; the engine stays in stretch mode until the
  next discontinuity, where 100 % returns to bypass.

**Feeding.** The stretch path works in sub-blocks of at most `SUB_BLOCK = 256` output frames. For `n` output frames at
speed `s` it feeds `m = floor(acc + n·s)` source frames (`acc` keeps the fraction), converted to stereo f32 (mono is
duplicated) in a preallocated buffer, and calls `process(in[..m], out[..n])`. The feed cursor runs `L_in` frames ahead
of what is heard. Sub-blocks are cut so that a loop end or the track end falls on a sub-block boundary (within one
frame), so a jump in the feed never sits inside a sub-block.

**Alignment (verified here, 2026-10-18).** To make the next output frame be song frame `T`: `reset()`, then `seek()`
with the pre-roll `source[T + L_in − PRE .. T + L_in]` (`PRE = block + interval` frames; zero-padded at the track
start), feed from `T + L_in`, and process-and-discard the first `L_out` output frames. This is how the library's own
`exact()` aligns, and a scratch test with 10 ms bursts confirmed it: the bursts came out within one 10 ms window of
`(burst − T) / s` at 50, 85 and 150 %. The discard costs about `L_out` frames of processing (well under 1 ms) once per
discontinuity, inside the callback. `L_in`/`L_out` come from the library (2646 frames each at 44.1 kHz, 1323 at
22.05 kHz).

**Song-time timeline.** The engine keeps a fixed ring (`TIMELINE = 64` entries, no allocation) of
`{out: output frame, song: f64 song frame, speed, kind: Normal | Wrap | End(kind), pass}`. Each processed sub-block
pushes `{out: out_count + L_out, song: feed_start − L_in, speed}`: the audio fed at that moment is heard `L_out` frames
later. The heard song position at output frame `o` is `e.song + (o − e.out) · e.speed` for the last entry with
`e.out ≤ o`. Everything the user perceives follows this heard position:
- `Shared.position` (so the playhead, the time box and `position_ms` are what is heard);
- **metronome clicks**: a beat of the click list sounds at the output frame where the heard position reaches it. The
  click is mixed after the stretcher, so it is never stretched and stays on the music's beats at any speed; its
  spacing follows the speed (120 BPM at 50 % clicks every 1 s, requirement 5);
- loop wraps (the `wraps` counter, the per-pass training speed, 2.6) and ends.

**Loop wrap without count-in.** The feed continues from the loop start (the existing 3 ms fade-out before the end and
fade-in after the start are applied to the fed samples), and a `Wrap` entry is pushed. The stretcher overlaps the two
ends itself, so there is no gap. The heard position wraps when output reaches that entry.

**Loop end with count-in, loop played once, track end.** The feed stops at the end (zeros after it) and an `End` entry
is pushed. When the heard position reaches it, the engine does what it does today at that point: start the count-in
(and, during it, align the stretcher at the loop start for the next pass's speed), stop at the loop start, or set
`ended`. So the last ~120 ms of a loop or a track are always heard.

**Count-in.** The pre-roll clicks are spaced by `60 / (bpm × speed)`: the manager computes Play and loop-Play count-ins
with the speed of the coming pass; for a wrap with count-in the engine computes it from the loop's song BPM (now carried
in `LoopSpec.count_in_bpm_milli`) and the next pass's speed (requirement 13). During a count-in in stretch mode the
stretcher is aligned at the start position, so the music starts exactly on the silent fourth beat.

**Jumps and scrubbing.** `Jump` aligns at the target and fades in over 3 ms (as today). `Seek` while playing stays silent
until `Resume`, which aligns and fades in. Section Start/End, Rew/FF and loop Prev/Next are all `Jump`s.

**Engine fields added:** `stretch: Stretcher` (created in `Engine::new`, which runs on the IPC thread), the input and
discard buffers, the timeline, `feed_pos`, `acc`, `out_count`, `mode`, `xfade_left`, `base_pct` (track speed), `speed`
(the ratio fed now), `run: Option<Run>` (2.6). `Shared` gains `speed_pct: AtomicU64` (the speed heard now) and
`pass: AtomicU64` (0 = no training run, else pass index + 1).

**New command:** `Cmd::SetSpeed(u32)` (track speed in %). Without a training run it becomes the fed speed at once.

### 2.5 Pure helpers (`src/speed.rs`)

```rust
pub const MIN_PCT: u32 = 25;
pub const MAX_PCT: u32 = 200;
pub const STEP_PCT: u32 = 5;
pub const SPEED_RANGE: &str = "The speed must be a whole number from 25 to 200 %";

/// `steps` presses of `-` (negative) or `+`: each moves to the next multiple of 5 in that direction; clamped.
pub fn step_pct(pct: u32, steps: i32) -> u32;
/// A typed speed: Ok for 25..=200, else Err(SPEED_RANGE).
pub fn check_pct(pct: i64) -> Result<u32, String>;
/// The stored factor (`player.json` "speed") → percent: round(speed × 100), clamped; not finite → 100.
pub fn pct_of_factor(speed: f64) -> u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Training { pub on: bool, pub start: u32, pub end: u32, pub step: u32, pub repeats: u32 }
// Default: on false, start 70, end 100, step 5, repeats 2.
impl Training {
    /// start/end 25..=200, step 1..=175, repeats 1..=99; the messages are in Task 2.
    pub fn validate(&self) -> Result<(), String>;
    /// Number of steps from start to end: 0 when equal, else ceil(|end − start| / step).
    pub fn steps(&self) -> u32;
    /// Speed of pass `pass` (0-based, counted from the loop Play): i = min(pass / repeats, steps);
    /// i == steps → end; else start ± i·step (towards end).
    pub fn pass_speed(&self, pass: u32) -> u32;
    /// What the Loops pane shows for that pass.
    pub fn progress(&self, pass: u32) -> Progress;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Progress { pub speed_pct: u32, pub repeat: u32, pub of: Option<u32> }
// pass / repeats < steps → { pass_speed, pass % repeats + 1, Some(repeats) }
// else (at the end speed, repeating for ever) → { end, pass − steps·repeats + 1, None }
```

The last step is clamped to the end: 60 → 85 in steps of 10 plays 60, 70, 80, 85. Start may be above end (slowing
down). Start = end plays at that speed from the first pass.

### 2.6 Speed training (requirements 8-13)

- **Settings** live in the loop (`LoopRec.training: Option<Training>`, 2.7): on/off, start, end, step, repeats, edited in
  the training popover (2.11) on any complete loop; switching Training on also switches the loop's Loop On on, and switching Loop On off
  switches Training off (training needs repeats; Q4).
- **A run** starts with the loop's own Play (the Loops pane button) when the current loop has Training on (Q3). Main
  Play passing through a repeating loop repeats it at the track speed, as today.
- **In the engine.** `LoopSpec` gains `training: Option<Plan>` (`Plan` = start, end, step, repeats). On `PlayLoop` with a
  plan the engine starts `Run { pass: 0 }` and feeds at `pass_speed(0)`. At each wrap the feed side does `pass += 1` and
  switches the fed speed to `pass_speed(pass)` exactly at the loop start, so each new speed applies from the loop start
  without a gap (requirement 9); the timeline entry carries `pass` and `speed`, so the heard side publishes them (and the
  clicks follow) when the wrap is heard. A run always plays in stretch mode, even at 100 %, so passes never switch paths.
- **A run ends** (requirement 11) on Pause, loop Stop, a `Seek` or `Jump` outside the loop, `SetLoop` with another loop,
  no loop, the loop's Training off or Loop On off, a new `PlayLoop` (which restarts at pass 0), a source change and
  closing. Then the fed speed returns to the track speed at once (while playing, through the normal ratio change), and
  `Shared.pass` becomes 0. Settings edited during a run apply from the next pass (the engine reads the new plan at the
  wrap, with its pass count).
- **While a run is active** the speed control and the readout show the training speed (`transport.speed_pct`), and the
  speed actions and `[`/`]` are refused ("Speed training is running; stop the loop to change the speed", Q5). The Loops
  pane's status line shows `Training 70 % · 2/4` (or `Training 80 % · 3` once at the end speed).
- **Absolute percentages** (spec open question 2, recommended): a 60 → 80 % run plays at 60..80 % whatever the track speed.

### 2.7 `player.json` schema 6 (requirements 7, 12; constraint "earlier files keep working")

- `"speed"` stays a factor in the file, now `pct / 100` with 0.25..=2.0 (two decimals). Reading: `pct_of_factor`, so
  schema 1-5 files (always `1.0`) open at 100 %, and a hand-edited `0.73` opens at 73 %.
- A loop may carry `"training": {"on": true, "start": 60, "end": 80, "step": 10, "repeats": 2}`, written once the user set
  it (absent = off with the defaults). Lenient read: a non-object, missing or out-of-range field drops the whole
  `training` (the loop itself stays).
- **Why a new schema:** a schema-5 build clamps `speed` to 0.5..1.5 and would write 0.25 back as 0.5. With schema 6 it
  opens such files read-only ("written by a newer version") and never overwrites them, as with every earlier bump.
  Training settings alone would have survived (loop extras are kept), but the bump covers both.
- Reads 1-6, writes 6. "Newer" test fixtures move from 6 to 7.

### 2.8 IPC

`player_tempo(id, action)`:
- `speed-step {steps}` (−1 / +1 per press; `step_pct`), `speed-reset` (100), new `speed-set {percent}` (`check_pct`).
  All return the snapshot, mark the state dirty (500 ms write) and send `Cmd::SetSpeed`. Refused during a training run.
  `NO_SPEED` and the "Enter a BPM first" refusal for speed go.

`player_loop(id, action)`: new `training {on, start, end, step, repeats}` for the current loop (validated; refused on
an incomplete loop with "Set the loop end first", like `flags`).

Types (Rust `player.rs`, mirrored in `ipc.ts`):
```ts
export interface Training { on: boolean; start: number; end: number; step: number; repeats: number }
export interface TrainingProgress { loop: number; speed_pct: number; repeat: number; of: number | null }
PlayerLoop      += training: Training                  // defaults when the file has none
PlayerTransport  : bpm      = song BPM of the section under the playhead, 0 = none (no longer × speed)
                 += played_bpm: number                  // bpm × speed_pct / 100, 0 = none
                 += speed_pct: number                   // the speed heard now (training speed during a run)
                 += training: TrainingProgress | null   // the running training, else null
PlayerSnapshot   : speed: number → speed_pct: number    // the track speed
                 -  can_change_speed
TempoAction     += { kind: 'speed-set'; percent: number }
LoopAction      += { kind: 'training'; on: boolean; start: number; end: number; step: number; repeats: number }
```
The transport event key gains `speed_pct` and `pass`, so a speed or pass change is sent at once (not only every 100 ms).

**Log lines.** Rust: `calliope: player speed id=<id> percent=<n>`; `calliope: player training loop=<i> pass=<n>
speed=<pct> repeat=<r>/<of|end>` (each heard pass, from the ticker, like `player wrap`); `calliope: player training
end loop=<i> speed=<pct>`. The existing `player count-in clicks=3 beat_ms=<ms>` shows the speed in `beat_ms`.
Frontend: `player speed percent=<n> via=<minus|plus|reset|typed|key>`, `player speed refused text=<t>`,
`player training index=<i> on=<b> start=<s> end=<e> step=<t> repeats=<r>`, `player readout bpm=<text|none>
speed=<pct>` (when the readout text changes), and `player active … speed=<pct>` (appended).

### 2.9 Layout (spec open question 1; owner: the popover alternative)

Measured facts: at 1280x800 with the tree folded the Player pane is 965 px wide; row 1 = Sections (496 px) + Loops
(424 px, three lines); row 2 = Playback (Rew, Play, FF, the time box, the position slider). Nothing scrolls. From the GUI
tests' fixed positions, the tablature placeholder (flex-1, `min-h-9`) is about 208 px high, so about 170 px of height
can go to the controls before the pane scrolls.

- **Speed group in the Playback pane** (`role="group" aria-label="Speed"`): "Speed", `-`, the box (whole number, `w-16`,
  `text-xl`), "%", `+`, Reset, then the readout ("60 BPM", `text-xl font-semibold`, fixed min width for "600 BPM",
  empty without a tempo map). Buttons keep the 3.25 rem height (used while playing). The group is `flex-none` inside the
  Playback flex-wrap row: it does not fit beside the transport at 965 px, so it wraps as one unit to a second line of
  the Playback pane (about +68 px); on wide windows it sits on the same line. The position slider keeps its 12 rem
  minimum.
- **"Training…" button on the Loops pane's third line**, after Play/Stop, Loop On and Count-In (3.25 rem high, `px-2`;
  the Play/Stop button goes from `w-20` to `w-16`, and the line's buttons from `px-3` to `px-2`, so the four fit the
  ~395 px inner width). It opens the training popover (2.11). It is filled amber while the loop's Training is on (same
  pressed style as Loop On), with the title "Speed training on: 60 → 80 %, +10 %, ×2" (or "Speed training settings");
  aria-label "Speed training settings"; disabled without a complete loop ("Set the loop end first").
- **Progress** goes into the existing reserved status line of the Loops pane ("Training 70 % · 2/4"; an error there
  wins), so nothing moves when a run starts.
- **Budget:** Playback +68 px, Loops +0 (no new line) ≈ 68 px of the ~170 px, so the target stays "overflow ≤ 14 px,
  `loops_beside=1` at 1280x800 tree folded" (expected 0; the tablature placeholder keeps about 140 px).
- **Fallbacks if the third line does not fit** (in this order): the button label "Train…" (the aria-label stays); then
  let the third line wrap (Loops +68 px, total ≈ 136 px, still within the budget). Never shrink the speed buttons.
- The BPM placeholders (`SHOW_PLACEHOLDERS`, `HINT`) in `BpmSection.svelte` are deleted.

### 2.10 What does not change

The Editor (its own engine, no speed), tempo editing in the Player, section/loop snapping, the click sound, device
latency handling (positions still lag the device by tens of ms; clicks are sample-aligned in the stream), MIDI (none
yet). The tablature and MIDI features later get song-time positions as before.

### 2.11 The training popover (owner Q1)

- **Component:** the shadcn-svelte `popover` (copied into `src/ui/lib/components/ui/popover/` like the other components,
  MIT, over bits-ui `Popover`), non-modal, anchored to the "Training…" button, `side="top"` with collision avoidance
  (flip/shift, 8 px padding) so it always stays inside the window; width 20 rem. bits-ui positions it through CSSOM
  (floating-ui), which the strict CSP allows; the GUI tests fail on any `csp-violation`, which checks it.
- **Content** (`TrainingPopover.svelte`, `role="dialog"` from bits-ui, aria-label "Speed training"): the title "Speed
  training" and the loop ("Loop 2 of 3"); a "Training on" switch (the existing shadcn `switch`); a 2×2 grid of labelled
  number boxes "Start %", "End %", "Step %", "Repeats" (`h-11`, `text-lg`, whole numbers, ranges of 2.5); a summary line
  ("60 → 80 %, +10 % every 2 passes"); a reserved one-line message slot for refusals; a "Close" button. About 340 px
  high, so it fits above the Loops pane at 1280x800 and inside the window at 1024x640 (where it may flip or shift).
- **Behaviour:** each box commits on Enter or when it loses the focus (as the loop boxes do), sending the whole
  `Training` with `player_loop training`; an invalid value is marked (`aria-invalid`), its message shows in the slot, and
  nothing is sent. The switch sends at once (on also turns Loop On on, 2.6). Edits during a run apply from the next pass.
- **Opening and closing:** click or Enter/Space on the button opens it and moves the focus to the switch; Tab and
  Shift+Tab walk switch → Start → End → Step → Repeats → Close (the focus is not trapped; Tab past Close leaves the
  popover and closes it). Escape closes it (an uncommitted box edit is dropped), so does a click outside (a valid
  uncommitted edit is committed by its blur first), and Close. On closing the focus returns to the "Training…" button.
  The Player's keys ignore the popover's controls (they are inputs and buttons, `OWN_KEYS`), so Space and letters work
  normally in it. It never opens by itself (no pop-up interrupting playback, `docs/ui.md`); playback goes on while it
  is open.
- **Logs** (frontend): `player training-popover open loop=<i> rect=<l>,<t>,<r>,<b> viewport=<w>x<h>` (after it is placed)
  and `player training-popover close via=<escape|outside|button|tab>`, so the GUI test checks it is inside the window and
  each way of closing.
- **Licence record:** add `popover` to the shadcn-svelte components row of `docs/licences.md`.

## 3. Tasks

Rust tasks end with `cargo clippy -p calliope-gui --all-targets -- -D warnings` clean for what they build, frontend
tasks with `npm run check` clean (both in `src/calliope-gui/`). Implementers run only the tests named in their task
(the orchestrator runs `npm test` and the GUI suite). **Extend the existing tests instead of adding near-duplicates**
(e.g. replace the old "speed not available" assertions, extend `ac13`/`ac28`; reuse `rig`, `rig_with`, `onsets`,
`clicks`, `goertzel`). **Keep test waits short**: ManualBackend pulls are instant, timing rules use injected `Instant`s,
GUI loops are one or two bars long, and `player.json` writes are waited for with the 500 ms debounce plus at most 1 s.
Never commit in a routine task.

### Task 1: Dependency, build profile, licence record, notices
- **files:** `src/calliope-gui/Cargo.toml`, `Cargo.toml` (root), `Cargo.lock`, `docs/licences.md`,
  `src/calliope-gui/third-party-notices.ts`, `src/calliope-gui/tests/frontend.rs`, `src/calliope-gui/README.md`
- **does:**
  - `[dependencies]` `signalsmith-stretch = "0.1.3"` (default features).
  - Root `Cargo.toml`: after the `flacenc` profile block add
    ```toml
    # Signalsmith Stretch is C++ compiled by the crate's build script; unoptimised it is too slow for the tests.
    [profile.dev.package.signalsmith-stretch]
    opt-level = 3
    ```
  - `docs/licences.md`, calliope-gui crate table, a row after `cpal`:
    `| \`signalsmith-stretch\` | dependency | \`0.1.3\` | 0.1.3 | MIT | time-stretch with the pitch kept (Player speed); Rust binding of the vendored Signalsmith Stretch C++ library |`
    (Locked from `Cargo.lock`, licence from `cargo metadata`.) Under the table: one paragraph saying it pulls in the `dasp`
    crates (MIT OR Apache-2.0) and, at build time only, `bindgen` (BSD-3-Clause), `clang-sys` (Apache-2.0) and `cc`, and
    needs `clang` (libclang) and a C++ compiler to build. "Bundled assets and copied source" gets one row:
    `Signalsmith Stretch 1.3.1 and signalsmith-linear, C++ sources vendored inside the signalsmith-stretch crate` | MIT |
    `Copyright (c) 2022 Geraint Luff / Signalsmith Audio Ltd and (c) 2025 Signalsmith Audio; compiled into the binary;
    both licence texts are in THIRD-PARTY-NOTICES.txt`.
  - `third-party-notices.ts`: crates may ship licence files in sub-folders. Add
    `const NESTED_LICENCES: Record<string, string[]> = { 'signalsmith-stretch': ['signalsmith-stretch/LICENSE.txt', 'signalsmith-linear/LICENSE.txt'] };`
    and include those files (name = the relative path) after the crate's own licence files; a listed file that is
    missing fails the build with a message naming it.
  - `tests/frontend.rs` `dist_ships_third_party_notices_for_every_linked_crate`: also assert the notices contain
    `signalsmith-stretch`, `Geraint Luff` and `Signalsmith Audio`.
  - `README.md` prerequisites: add `clang` to the pacman line, with "(libclang, for the time-stretch binding)".
- **done when:** `cargo build -p calliope-gui` succeeds; `cd src/calliope-gui && npm run build` succeeds;
  `cargo test -p calliope-gui --test frontend` passes (licence record and notices tests).
- **routine:** no

### Task 2: Pure speed rules (`speed.rs`)
- **files:** `src/calliope-gui/src/speed.rs` (new), `src/calliope-gui/src/lib.rs`
- **does:** exactly the API of 2.5, `pub mod speed;` in `lib.rs` (keep the module list's order style). Formulas:
  - `step_pct`: start from `pct.clamp(25, 200)`; for each `+` step `p = (p / 5 + 1) * 5`; for each `-` step
    `p = ((p + 4) / 5 - 1) * 5`; clamp to 25..=200 after each step.
  - `check_pct`: `25..=200` → `Ok(pct as u32)`, else `Err(SPEED_RANGE.to_string())`.
  - `pct_of_factor`: not finite → 100; else `(speed * 100.0).round()` clamped to 25..=200.
  - `validate` messages: start or end outside 25..=200 → `"Training speeds must be between 25 and 200 %"`; step outside
    1..=175 → `"The step must be between 1 and 175 %"`; repeats outside 1..=99 → `"Repeats must be between 1 and 99"`.
  - `steps`, `pass_speed`, `progress` as in 2.5 (integer maths only; `ceil(a / b)` is `(a + b - 1) / b`).
- **tests** (in `speed.rs`, `#[cfg(test)]`):
  - `step_pct`: (100, −3) → 85; (73, −1) → 70; (73, 1) → 75; (70, 1) → 75; (70, −1) → 65; (25, −1) → 25; (27, −1) → 25;
    (200, 1) → 200; (198, 1) → 200; (100, 0) → 100; (300, −1) → 195.
  - `check_pct`: 25 and 200 ok; 20, 24, 201, 250, −5 → `Err(SPEED_RANGE)`.
  - `pct_of_factor`: 1.0 → 100; 0.8 → 80; 0.73 → 73; 0.1 → 25; 9.0 → 200; NaN → 100.
  - `Training::default()` = {false, 70, 100, 5, 2}; `validate` ok for the default and for (60, 80, 10, 2); each message
    for start 20, end 201, step 0, step 176, repeats 0, repeats 100.
  - (60, 80, 10, 2): `steps` 2; `pass_speed` 0..=6 → 60, 60, 70, 70, 80, 80, 80; `progress` 0..=6 →
    (60,1,Some(2)), (60,2,Some(2)), (70,1,Some(2)), (70,2,Some(2)), (80,1,None), (80,2,None), (80,3,None).
  - (100, 60, 15, 1): `steps` 3; passes 0..=4 → 100, 85, 70, 60, 60.
  - (60, 85, 10, 2): passes 0..=7 → 60, 60, 70, 70, 80, 80, 85, 85.
  - (80, 80, 5, 3): `steps` 0; pass 0 → 80 and progress (80, 1, None); pass 4 → (80, 5, None).
- **done when:** `cd src/calliope-gui && cargo test -p calliope-gui --lib speed` passes and
  `cargo clippy -p calliope-gui --lib --tests -- -D warnings` is clean. Run it once, then stop. Do not commit.
- **routine:** yes

### Task 3: Stretcher wrapper (`stretch.rs`)
- **files:** `src/calliope-gui/src/stretch.rs` (new), `src/calliope-gui/src/lib.rs`
- **does:** `pub(crate) struct Stretcher` around `signalsmith_stretch::Stretch::preset_default(2, rate)`:
  `new(rate)` (also does one `seek` of silence so no later call grows a buffer), `input_latency()`, `output_latency()`,
  `pre_roll_frames()` (block + interval: `rate*0.12 + rate*0.03` as the preset computes it, or a value that is at least
  that), `reset()`, `seek(pre_roll: &[f32], speed: f64)`, `process(input: &[f32], output: &mut [f32])` (interleaved
  stereo), and `align(&mut self, source: impl Fn(u64, &mut [f32]), target: u64, speed: f64, scratch: &mut [f32])`
  implementing 2.4's alignment (the closure fills stereo frames from a song frame, zeros outside the track; the caller
  then feeds from `target + input_latency()`). No allocation outside `new`.
- **tests** (in `stretch.rs`, synthetic signals at 22050 Hz and 44100 Hz):
  - a 440 Hz stereo tone at 25, 50, 150 and 200 %: the dominant frequency of 1 s of steady output is 440 Hz ± 2 Hz
    (± 3 Hz at 25 %; zero crossings or a Goertzel scan 400..480 Hz in 0.5 Hz steps);
  - feeding 10 s of input at 85 % in 256-frame blocks gives 11.76 s ± 1 % of output;
  - alignment: a source of 10 ms 1 kHz bursts every 0.5 s, aligned at a non-round frame `T`; at 50, 85 and 150 % the
    peak of each burst's 1 ms envelope in the output is within ±10 ms of `(burst_end_mid − T) / s` (the measured
    behaviour of 2.4); the first output frames after `align` are not the pre-roll's audio (silent up to the first burst).
- **done when:** `cargo test -p calliope-gui --lib stretch` passes; clippy clean.
- **routine:** no

### Task 4: `player.json` schema 6, training settings, speed kept on tempo changes
- **files:** `src/calliope-gui/src/player_state.rs`, `src/calliope-gui/src/tempo_store.rs`,
  `src/calliope-gui/src/beat_detect_tests.rs`
- **does:**
  - `SCHEMA = 6`; `MIN_SPEED`/`MAX_SPEED` become 0.25/2.0; `from_value` stores `speed = pct_of_factor(v) / 100`
    (exactly `pct as f64 / 100.0`); `PlayerState::speed_pct()` and `set_speed_pct(u32)`.
  - `LoopRec.training: Option<Training>` (typed, `skip_serializing_if = "Option::is_none"`), read leniently (2.7) and
    removed from `extra`.
  - `PlayerState::set_training(&mut self, t: Training) -> Result<(), String>` on the current loop: incomplete → "Set the
    loop end first"; `t.validate()?`; `t.on` → also `repeat = true`. `flags(Some(false), _)` (Loop On off) also sets
    `training.on = false` when present.
  - `tempo_store::replace` no longer sets `speed` to 1.0 (doc comment and its test `…_and_resets_speed` → `…_and_keeps_speed`).
  - Test literals: "written as 5" → 6 (`player_state.rs` lines ~501, 549, 588, 807; `beat_detect_tests.rs` ~146, 202);
    "newer" fixtures 6 → 7 (`player_state.rs` ~672, `tempo_store.rs` ~100). Fixtures written as 5 that are read stay.
- **tests** (extend the existing ones in `player_state.rs`):
  - `lenient_read`: `speed` 9.0 → 2.0, −1 → 0.25, "fast" → 1.0; a schema-5 file with `speed: 1.0` → 100 %, 0.73 → 73 %;
  - round trip: speed 0.85 and a loop with training (60, 80, 10, 2, on) are written and read back equal; a loop without
    training writes no `training` key; a loop with `"training": {"start": 10}` or `"training": 3` loads without it;
  - `set_training`: refused on an incomplete loop and with an invalid value (state unchanged); `on` switches Loop On on;
    `flags` Loop On off switches training off;
  - the written file is schema 6; a schema-7 file is read-only.
- **done when:** `cargo test -p calliope-gui --lib player_state tempo_store beat_detect` passes; clippy clean.
- **routine:** no

### Task 5: Engine stretch mode and `Cmd::SetSpeed`
- **orchestrator note (after Task 3):** `Stretcher::align` returns the song frame to feed next (it consumes source
  audio while discarding the first `output_latency()` frames), so continue from that value, not from
  `target + input_latency()`. `align`'s scratch buffer must hold `pre_roll_frames()` stereo frames (tests use
  `pre_roll_frames() * 2 * 2` floats). Remove the temporary `#[allow(dead_code)]` on `mod stretch` in `lib.rs`.
- **files:** `src/calliope-gui/src/player.rs`, `src/calliope-gui/src/player_tests.rs`,
  `src/calliope-gui/tests/player_realtime.rs` (new)
- **does:** 2.4 completely, without the training run: the two modes, `Cmd::SetSpeed`, the feed with sub-blocks cut at
  loop/track ends, alignment, the timeline ring, the crossfade, clicks and wraps from the heard position, the
  `End` handling (count-in at a wrap, loop played once, track end), count-in alignment, `Shared.speed_pct`.
  `LoopSpec.count_in: Option<CountIn>` becomes `count_in_bpm_milli: Option<u32>` (the loop start's song BPM × 1000) and
  the engine builds the wrap count-in with `count_in_plan(bpm × speed)`. The manager side only needs the minimum to
  compile and keep today's behaviour (`loop_spec` fills the new field; `install` sends `SetSpeed(state.speed_pct())`).
  For the real-time test, mark `Engine`, `Cmd`, `Shared`, `LoopSpec`, `Engine::new/apply/render` and the `Pcm`
  constructor `#[doc(hidden)] pub` (the existing pattern for test doubles).
- **tests:**
  - **`player_tests.rs`** (Engine driven directly with a synthetic `Pcm`: 300 Hz 10 ms bursts on every beat of 120 BPM;
    the "click render" uses an all-zero `Pcm` of the same length with the beat grid, the "music render" the burst `Pcm`
    with an empty grid and identical commands, so the two timelines are the same):
    - at 50, 85 and 150 %, from Play and after a `Jump`, a loop wrap and a speed change while playing: each click onset
      (first non-zero frame) is within ±10 ms of its burst's envelope peak (5 ms windows), and click spacing is
      `60/(120·s)` s ± 1 frame;
    - speed 100 % output is still bit-identical to the source (the existing exact tests stay untouched and pass);
    - 100 % → 70 % while playing a steady tone: no 10 ms window below 50 % of the tone's RMS, `Shared.position` never
      decreases (64-frame pulls), and it advances 0.7 s ± 70 ms per output second once settled;
    - a repeating loop at 80 %: wraps counted, the heard position goes back to the loop start, no window below 50 % RMS
      across the wrap;
    - loop end with count-in at 50 %: three clicks 1000 ms apart (120 BPM), then the burst at the loop start on the
      silent fourth beat (±10 ms);
    - the track end at 150 %: the last 50 ms of the source are heard (tone present), then `ended`;
    - `Seek` while playing at 85 % stays silent until `Resume`, then the audio is from the seek target (burst alignment).
  - **`tests/player_realtime.rs`**: a `#[global_allocator]` that counts allocations on the current thread; build an
    engine at 85 % (allocations allowed), then render 3 s in 512-frame blocks with a speed change, a `Jump`, a loop wrap,
    a wrap with count-in, `SetGrid`, `SetLoop` and the track end: zero allocations during `render`/`apply` (grids are
    built before, as the manager does). Note in a comment that C++ allocations are not seen by this test.
- **done when:** `cargo test -p calliope-gui --lib player::tests` and `cargo test -p calliope-gui --test player_realtime`
  pass; clippy clean.
- **routine:** no

### Task 6: Training run in the engine
- **files:** `src/calliope-gui/src/player.rs`, `src/calliope-gui/src/player_tests.rs`
- **does:** 2.6's engine part: `LoopSpec.training: Option<speed::Plan>` (start, end, step, repeats; `Copy`), the run on
  `PlayLoop`, the per-pass speed switched at the wrap on the feed side, `pass` and `speed` carried by the timeline and
  published in `Shared.pass`/`Shared.speed_pct` when heard, the end rules (Pause, Seek/Jump outside the loop, `SetLoop`
  with another loop, none, or the same loop without training or without repeat, new `PlayLoop`), the return to the
  track speed, the per-pass count-in speed, and "a run always uses stretch mode".
- **tests** (`player_tests.rs`, a synthetic 2 s loop, Engine driven directly; extend Task 5's helpers):
  - plan (60, 80, 10, 2): the output length of passes 1-6 is `2 s / 0.6`, `/0.6`, `/0.7`, `/0.7`, `/0.8`, `/0.8` ± 2 %
    (pass boundaries = `wraps` changes, 64-frame pulls); `Shared.pass` 1..6 and `speed_pct` 60, 60, 70, 70, 80, 80 as each
    pass is heard; no 10 ms window below 50 % RMS at any pass change;
  - with the loop's count-in on: before every pass three clicks at `60/(120·pass_speed)` s;
  - Pause in pass 3 → `pass` 0 and `speed_pct` = track speed; the next `PlayLoop` starts at 60 % again;
  - `Jump` outside the loop ends the run and playback continues at the track speed; a `Jump` inside the loop keeps it;
  - `SetLoop` with training removed mid-pass → the track speed at once; with changed settings → used from the next pass;
  - plan (100, 100, 5, 1): stays in stretch mode, all passes 2 s long ± 2 %.
- **done when:** `cargo test -p calliope-gui --lib player::tests` passes; clippy clean.
- **routine:** no

### Task 7: Manager and IPC: speed actions, transport fields, training action, logs
- **files:** `src/calliope-gui/src/player.rs`, `src/calliope-gui/src/player_ipc.rs`, `src/calliope-gui/src/player_tests.rs`,
  `src/calliope-gui/src/player_ipc_tests.rs`, `src/calliope-gui/tests/acceptance_player_backend.rs` (three lines only)
- **does:**
  - Keep the integration tests compiling: in `acceptance_player_backend.rs` replace the two lines at ~242-243
    (`speed_action`, `can_change_speed`) with `assert!(r.m.speed(&r.id(), SpeedAction::Step(-1), Instant::now()).is_ok());`
    and the line at ~1227 with `assert!((25..=200).contains(&s.speed_pct));`. Task 9 extends these.
  - `pub enum SpeedAction { Step(i32), Reset, Set(i64) }` and `PlayerManager::speed(&mut self, id, SpeedAction, now)`:
    refused during a run; otherwise update `state` (`speed_pct`), `send(SetSpeed)`, mark dirty, log
    `calliope: player speed id=<id> percent=<n>`, return the snapshot. Remove `speed_action`, `NO_SPEED`.
  - `LoopAction::Training(Training)` → `state.set_training`, `sync_loop`, mark dirty.
  - `loop_spec` fills `training` (when on and repeating) and `count_in_bpm_milli`; `plan_at(ms, pct)` takes the speed:
    Play uses the track speed, loop Play the first pass speed (training start, else the track speed).
  - `transport_state`: `bpm` unscaled, `played_bpm`, `speed_pct` (from `Shared.speed_pct` while an output is open, else
    the track speed), `training` (`Progress` of the current loop for `pass − 1`, with the loop index). `EventKey` gains
    `speed_pct` and `pass`.
  - `snapshot`: `speed_pct` (track) replaces `speed`; `can_change_speed` removed; `PlayerLoop.training` (defaults when
    none).
  - `tick`: the training log lines of 2.8 when `pass` changes and when a run ends.
  - `set_bpm` keeps the speed.
  - `player_ipc.rs`: `TempoAction::SpeedSet { percent: i64 }`; `SpeedStep`/`SpeedReset`/`SpeedSet` → `m.speed(…)`;
    `LoopCmd::Training { on, start, end, step, repeats }` → `LoopAction::Training`.
- **tests:**
  - **`player_tests.rs`:** replace `section_flags_and_speed_are_refused_without_a_tempo`'s speed part with: without a
    tempo `speed-step −1` works (95) and `played_bpm` is 0; replace `tempo_change_keeps_loops_in_time_and_resets_speed`
    with `…_and_keeps_speed` (80 stays 80); the transport `bpm` assertion at ~1144 drops `× speed` and a new one checks
    `played_bpm = bpm × 0.5` at 50 %; steps clamp at 25/200; `speed-set 250` refused, unchanged; during a run every speed
    action is refused and `transport.training` is `{loop, 60, 1, Some(2)}` in pass 1; the 500 ms write has `speed: 0.8`;
    loop Play with training and Count-In → the count-in log has `beat_ms=833` at 120 BPM and 60 %.
  - **`player_ipc_tests.rs`:** `speed-set` and `training` deserialize; the existing `speed-step`/`speed-reset` tests stay.
- **done when:** `cargo test -p calliope-gui --lib player` passes (all `player*` modules); clippy clean.
- **routine:** no

### Task 8: Schema-number literals in the integration tests
- **files:** `src/calliope-gui/tests/gui_editor_e2e.rs`, `src/calliope-gui/tests/gui_player_e2e.rs`,
  `src/calliope-gui/tests/acceptance_beat_autodetect.rs`, `src/calliope-gui/tests/acceptance_player_backend.rs`
- **does:** exact edits; line numbers are from `e49615e` and may be a few lines off, so find each line by its text.
  1. "Written as 5" becomes "written as 6": in each line below replace the `5` inside `["schema_version"], 5` with `6`:
     - `gui_editor_e2e.rs:361`; `gui_player_e2e.rs:361`, `:428`, `:527`, `:958`;
     - `acceptance_beat_autodetect.rs:460`, `:713`;
     - `acceptance_player_backend.rs:1202`, `:1792`, `:2006`, `:2036`, `:2066`, `:2415`, `:2443`, `:2920`, `:3198`, `:3436`.
  2. "Newer" fixtures: replace `"schema_version":6` with `"schema_version":7` in `acceptance_beat_autodetect.rs:611`
     and `acceptance_player_backend.rs:1265`, `:2071`, `:2446`. In `acceptance_player_backend.rs:3221` change
     `for v in [6u64, 7, 99, 4_000_000_000]` to `for v in [7u64, 8, 99, 4_000_000_000]`.
  3. The speed is now kept by a new tempo map (Task 4): in `acceptance_beat_autodetect.rs:462` replace the line
     `    assert_eq!(v["speed"], 1.0, "speed is 100 % after a new map");` with
     `    assert_eq!(v["speed"], 0.7, "the speed is kept with a new map");`.
  4. Leave every other `schema_version` alone (files read as 1-5 test the migration).
- **done when:** `cd src/calliope-gui && cargo test -p calliope-gui --test acceptance_beat_autodetect && cargo test -p
  calliope-gui --test acceptance_player_backend --no-run && cargo test -p calliope-gui --features e2e-hooks --test gui_editor_e2e --test
  gui_player_e2e --no-run` succeeds. Run it once, then stop. Do not commit.
- **routine:** yes

### Task 9: Backend acceptance tests for the spec's criteria
- **files:** `src/calliope-gui/tests/acceptance_player_backend.rs`
- **does:** extend `ac13` (patched in Task 7): at BPM 0 the speed works and `played_bpm` is 0; then a new section
  "gui-player-speed" with these tests (ManualBackend, the library-player fixture at 22050 Hz, 64-frame pulls where
  timing matters):
  - `sp01_minus_three_times_is_85_and_10s_take_11_8s`: 100 % → three `speed-step −1` → `speed_pct` 85; play from 1.0 s;
    output frames until the position reaches 11.0 s = 11.76 s ± 1 %.
  - `sp02_a_440hz_tone_keeps_its_pitch_at_50_and_150`: the 440 Hz source (`stem:vocals`); 1 s of steady output after
    0.5 s at 50 % and at 150 %: dominant frequency 440 ± 2 Hz.
  - `sp03_changing_to_70_while_playing_has_no_pause_and_keeps_moving`: play 1 s at 100 %, `speed-set 70`, pull 2 s: no
    10 ms window below 50 % of the tone's RMS; the position never decreases; it advanced 1.4 s ± 70 ms.
  - `sp04_120bpm_at_50_shows_60_and_clicks_every_second_on_the_beats`: the TS fixture (120 BPM from 500 ms, Click on in
    section 1), 50 %: `played_bpm` 60, `bpm` 120; click onsets 1000 ms ± 1 ms apart; in each 64-frame block that holds an
    onset the song position is within 2 ms of a beat of the map.
  - `sp05_no_tempo_map_no_bpm_and_speed_still_works`: Practice Song (no map): `played_bpm` 0; at 50 % the position
    advances 1.0 s ± 2 % per 2 s of output.
  - `sp06_20_and_250_are_refused_and_the_speed_is_unchanged`: from 80 %: `speed-set 20` and `250` → `Err(SPEED_RANGE)`,
    still 80 %, nothing written.
  - `sp07_80_percent_survives_close_and_reopen`: `speed-set 80`, 600 ms tick → `player.json` `speed` 0.8, schema 6;
    close, reopen → `speed_pct` 80 and playback at 80 % (position rate ± 2 %).
  - `sp08_training_60_to_80_step_10_two_repeats`: a 2 s loop (bar 2 to bar 3 of the TS fixture), training on → loop Play:
    pass lengths as in Task 6 ± 2 %; `transport.training` (60,1,2), (60,2,2), (70,1,2), (70,2,2), (80,1,null),
    (80,2,null) and `speed_pct` with it.
  - `sp09_stopping_returns_to_the_track_speed_and_the_next_play_restarts`: track 90 %, stop the loop in pass 3 →
    `training` null, `speed_pct` 90; loop Play → pass 1 at 60 % (length `2 s / 0.6` ± 2 %).
  - `sp10_training_settings_survive_close_and_reopen`: settings (60, 80, 10, 2, on) on loop 2 of 3 → `player.json`
    `loops[1].training` equal; reopen → `snapshot.loops[1].training` equal; the other loops have none (defaults shown).
  - `sp11_previous_version_file_loads_at_100_unchanged`: a schema-5 file with `speed: 1.0`, three loops and two named
    sections with flags → `speed_pct` 100, loops (ids, boundaries, flags, current) and sections (starts, BPM, names,
    flags) equal to what the previous build showed (compare with the snapshot taken before this feature's changes are
    applied to the same JSON: hard-code the expected values); after one edit it is written as 6 with everything else
    unchanged.
  - `sp12_count_in_during_training_uses_the_current_speed`: loop Count-In on, plan (60, 80, 10, 1) at 120 BPM: the
    count-in before pass 1 has 833.3 ms beats (`player count-in … beat_ms=833`), before pass 2 714.3 ms (± 1 frame).
- **done when:** `cargo test -p calliope-gui --test acceptance_player_backend` passes; `cargo test --workspace` passes;
  `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- **routine:** no

### Task 10: TS IPC types and test fixtures
- **files:** `src/calliope-gui/src/ui/lib/ipc.ts`, `src/calliope-gui/src/ui/lib/ipc-player.test.ts`, and the snapshot /
  transport fixtures in `src/calliope-gui/src/ui/lib/player-state.test.ts`,
  `src/calliope-gui/src/ui/components/player/ControlBar.test.ts`, `src/calliope-gui/src/ui/acceptance_player.test.ts`,
  `src/calliope-gui/src/ui/views/PlayerView.test.ts`
- **does:** the types of 2.8 (with doc comments as in the file); the fixtures get `speed_pct: 100` instead of `speed: 1`,
  lose `can_change_speed`, and every transport gets `played_bpm` (= `bpm` at 100 %), `speed_pct: 100`, `training: null`;
  every loop gets `training: { on: false, start: 70, end: 100, step: 5, repeats: 2 }`. Prefer changing the shared fixture
  helpers of each file over editing each literal.
- **tests:** `ipc-player.test.ts`: `playerTempo(id, {kind:'speed-set', percent: 85})` and `playerLoop(id, {kind:'training',
  …})` send exactly those payloads.
- **done when:** `cd src/calliope-gui && npx vitest run src/ui/` passes and `npm run check` is clean.
- **routine:** no

### Task 11: Frontend speed formatting helpers
- **files:** `src/calliope-gui/src/ui/lib/speed-format.ts` (new), `src/calliope-gui/src/ui/lib/speed-format.test.ts` (new)
- **does:**
  ```ts
  import type { TrainingProgress } from './ipc';
  export const MIN_SPEED = 25;
  export const MAX_SPEED = 200;
  /** A typed speed: whole number 25..200, optional "%" and spaces; else null. */
  export function parseSpeed(text: string): number | null; // /^\s*(\d{1,3})\s*%?\s*$/ then range check
  /** "60 BPM" (formatBpm of the value), or "" for 0. */
  export function readoutText(playedBpm: number): string;
  /** "70 % · 2/4", or "80 % · 3" at the end speed (of === null). */
  export function formatTraining(p: TrainingProgress): string;
  /** A training box value: whole number within [min, max]; else null. */
  export function parseTrainingField(text: string, min: number, max: number): number | null;
  ```
  `readoutText` uses `formatBpm` from `./bpm-format` (integers plain, else one decimal). The `·` is U+00B7 with one space
  on each side; `%` follows the number after one space.
- **tests** (`speed-format.test.ts`): `parseSpeed`: "85" → 85, " 85 % " → 85, "85%" → 85, "25" → 25, "200" → 200, "20",
  "250", "85.5", "", "abc", "-5" → null. `readoutText`: 60 → "60 BPM", 79.05 → "79.1 BPM", 0 → "". `formatTraining`:
  {speed_pct 70, repeat 2, of 4} → "70 % · 2/4"; {80, 3, null} → "80 % · 3". `parseTrainingField("10", 1, 175)` → 10;
  ("0", 1, 175) → null; ("176", 1, 175) → null; ("x", 1, 99) → null.
- **done when:** `cd src/calliope-gui && npx vitest run src/ui/lib/speed-format.test.ts` passes and `npm run check` is
  clean. Run it once, then stop. Do not commit.
- **routine:** yes

### Task 12: Frontend Player state for speed and training
- **files:** `src/calliope-gui/src/ui/lib/player-state.svelte.ts`, `src/calliope-gui/src/ui/lib/player-state.test.ts`
- **does:** `speedStep(steps, via)`, `speedReset()`, `speedSet(text)` (parses with `parseSpeed`; refused text logs
  `player speed refused text=<t>` and sets `pl.speedError` = "Enter a whole number from 25 to 200 %" without calling
  IPC), `setTraining(t)`; `speedLocked()` (a run is active); the log lines of 2.8 (`player speed …`, `player training
  …`, `player readout …` when the readout text changes, `speed=<pct>` on `player active`). Errors from Rust go to
  `pl.speedError` (speed) and `pl.actionError` (training).
- **tests** (`player-state.test.ts`): each function sends the right action and logs once; a refused typed value sends
  nothing; the readout log fires only on a change; during a run (`transport.training` set) `speedStep` sends nothing.
- **done when:** `npx vitest run src/ui/lib/player-state.test.ts` passes; `npm run check` clean.
- **routine:** no

### Task 13: Speed group in the Playback pane, keys
- **files:** `src/calliope-gui/src/ui/components/player/SpeedControl.svelte` (new), `PlaybackSection.svelte`,
  `BpmSection.svelte`, `src/calliope-gui/src/ui/views/PlayerView.svelte`, `ControlBar.test.ts`,
  `src/calliope-gui/src/ui/views/PlayerView.test.ts`, `src/calliope-gui/src/ui/acceptance_player.test.ts`
- **does:** 2.9's Speed group (labels: `-` aria-label "Slower", `+` "Faster", box "Speed in percent", readout
  `data-testid="bpm-readout"` aria-label "Playing BPM"); disabled with reasons as titles: not active, at 25 (`-`) / 200
  (`+`) / 100 (Reset), during a run ("Speed training is running; stop the loop to change the speed"). The box shows
  `transport.speed_pct`; Enter or leaving commits, Escape reverts; an invalid value shows `aria-invalid`, the message as
  title, and "25–200 %" in the readout slot in the destructive colour until the next edit (no layout shift). Delete the
  BPM placeholders in `BpmSection.svelte`; it shows the song BPM. Keys `[`/`]` in `PlayerView.svelte` (ignored on
  auto-repeat, passed through the waveform like Space).
- **tests:**
  - `ControlBar.test.ts`: `-` ×3 from 100 sends three `speed-step −1`; typing "70" + Enter sends `speed-set 70`;
    "20" sends nothing and marks the box invalid; Reset disabled at 100; all speed controls disabled during a run; the
    readout shows "60 BPM" for `played_bpm` 60 and is empty for 0; the BPM box shows `bpm` (song), not `played_bpm`.
  - `PlayerView.test.ts`: `[` and `]` send `speed-step −1`/`+1`; a repeated keydown sends nothing; with the waveform
    focused they still work.
  - `acceptance_player.test.ts`: the Playback group contains the Speed group after the position slider; no
    "Comes in a later update" text anywhere.
- **done when:** `npx vitest run src/ui/components/player/ src/ui/views/PlayerView.test.ts src/ui/acceptance_player.test.ts`
  passes; `npm run check` clean; a `gui-shot` at 1280x800 (tree folded, `npm run dev:app` on `:1`) shows the Speed group
  on the Playback pane's second line with nothing overlapping.
- **routine:** no

### Task 14: "Training…" button and the training popover
- **files:** `src/calliope-gui/src/ui/lib/components/ui/popover/*` (new, shadcn-svelte popover copied as for the other
  components), `src/calliope-gui/src/ui/components/player/TrainingPopover.svelte` (new),
  `src/calliope-gui/src/ui/components/player/LoopsSection.svelte`, `ControlBar.test.ts`,
  `src/calliope-gui/src/ui/acceptance_player.test.ts`, `docs/licences.md` (`popover` in the shadcn-svelte row),
  `components.json` only if the CLI needs it
- **does:** 2.9's button and third-line sizing, 2.11's popover (content, commits, refusals, open/close rules, focus,
  logs) and the progress text in the Loops status line. Then measure at 1280x800 with the tree folded (`player layout`
  log) and apply 2.9's fallbacks until the button fits on the third line, `loops_beside=1` and overflow ≤ 14 px; open
  the popover at 1280x800 and 1024x640 (tree folded and shown) and check its `rect` log is inside the viewport.
- **tests:**
  - `ControlBar.test.ts`: the button is disabled without a complete loop; it is amber with the summary title when
    training is on; clicking it opens the popover with the focus on the switch; the switch sends `training` with
    `on: true` and the current values; typing `60`, `80`, `10`, `2` (Enter or Tab) sends them; "0" in Step marks the box
    invalid, shows the message and sends nothing; Escape closes it, drops an uncommitted edit and returns the focus to
    the button; a click outside closes it (a valid pending edit is sent once); Close closes it; Tab from Close leaves and
    closes it; each close logs its `via`; with `transport.training` {70, 2, 4} the status line reads
    "Training 70 % · 2/4" and an action error replaces it; the status line keeps its height when the text appears.
  - `acceptance_player.test.ts`: the Loops group's third line is Play, Loop On, Count-In, "Training…"; the Loops group
    has no fourth line; the popover is not in the DOM until opened.
- **done when:** `npx vitest run src/ui/components/player/ src/ui/acceptance_player.test.ts` passes; `npm run check`
  clean; `cargo test -p calliope-gui --test frontend` passes (licence record, no static `style=`); the measured
  `overflow`, `width` and `loops_beside` at 1280x800 tree folded and the popover rects at both sizes are reported.
- **routine:** no

### Task 15: GUI end-to-end, positions, visual review, docs
- **files:** `src/calliope-gui/tests/gui_player_e2e.rs`, `src/calliope-gui/tests/gui_player_acceptance_e2e.rs`,
  `docs/ui.md`
- **does:**
  - Re-measure every fixed position in both files at 1280x800 with the tree folded (everything above the Playback pane
    moves up by the Playback pane's second line, and the Loops third line's buttons narrow: `LOOP_PLAY`, `LOOP_ON` and
    `LOOP_COUNT_IN` change); add `SPEED_MINUS`, `SPEED_PLUS`, `SPEED_BOX`, `SPEED_RESET` and `TRAIN_OPEN` (the
    "Training…" button). The popover's fields are reached with the keyboard (Tab order of 2.11), not by fixed positions,
    so its placement can flip without breaking the test.
  - New test `player_speed` (capture WAV, the With Tempo fixture written as schema 6 with section 1 Click on, the 440 Hz
    source if the track has one, else its own tone):
    1. layout line: `overflow` ≤ 14, `loops_beside=1`; screenshot `player-speed-layout`;
    2. `SPEED_MINUS` ×3 → `player speed percent=85 via=minus` ×3 ending at 85; Play from 1.0 s for 3 s → the position
       advanced 2.55 s ± 0.15 s (transport lines); the capture's tone is at its frequency ± 3 Hz;
    3. type 50 + Enter → `percent=50 via=typed`, `player readout bpm=60 BPM speed=50`; the click onsets in the capture
       are 1000 ms ± 5 ms apart;
    4. type 20 → `player speed refused text=20`, then 250 → refused; still 50 (`player.json` after 1 s: `speed` 0.5);
    5. `]` → 55; Reset → 100;
    6. set a 1-bar loop (bar 2 to bar 3); click `TRAIN_OPEN` → `player training-popover open … rect=… viewport=…` with the
       rect inside the viewport; screenshot `player-training-popover`; Space on the focused switch, then Tab and type
       60, 70, 10, 1 (Tab after each) → `player training index=0 on=true start=60 end=70 step=10 repeats=1`; Escape →
       `close via=escape` and the focus is back on the button (a following Space opens it again: `open`); click on the
       waveform's blank area → `close via=outside`;
    7. loop Play → `calliope: player training loop=0 pass=1 speed=60 repeat=1/1`, then `pass=2 speed=70 repeat=1/end`;
       the status line screenshot shows "Training 70 % · 1"; loop Stop → `player training end loop=0 speed=100`;
    8. type 80 in the speed box, restart the app → `player active … speed=80`, `player.json` `loops[0].training` is
       restored, and the reopened popover shows 60/70/10/1 (screenshot); at 1024x640 (tree shown) open it again → the
       rect is inside the viewport.
  - `gui_player_acceptance_e2e.rs`: update positions only (and schema literals already done in Task 8).
  - `visual_review`: the new Speed group, the "Training…" button and the open popover in both themes, 1280x800 and 1024x640, tree folded and shown;
    report `overflow`/`loops_beside` each; judge readability at 1-2 m, the readout and progress text, and that nothing
    moves when a run starts or an invalid value is typed, and that the popover is fully visible and readable.
  - `docs/ui.md` "Decided by the team": the Speed group (place, steps, typing, Reset, readout, disabled during training),
    the "Training…" button and popover (open/close rules, focus), the progress text, `[`/`]`, the measured layout
    numbers.
- **done when:** `cd src/calliope-gui && npm run test:gui -- player_speed` passes (private display); screenshots
  reviewed and the measured values reported.
- **routine:** no

### Task 16: README Player usage
- **files:** `src/calliope-gui/README.md`
- **does:** in the "## Player" section add one bullet after the Loops bullet, nothing else:
  ```text
  * Speed (Playback pane): "-" and "+" change the speed in 5 % steps from 25 % to 200 % (keys "[" and "]"), the
    box takes a whole number, Reset returns to 100 %. The pitch stays the same, positions and loops stay in song
    time, and the metronome follows. With a tempo the pane shows the BPM being played. Speed training (Loops
    pane): set start %, end %, step % and repeats for a loop and switch Training on; the loop's Play then plays it
    at the start speed and moves one step after the given repeats until the end speed. Stopping the loop returns
    to the track speed. Both are saved with the track.
  ```
- **done when:** `git diff --stat -- src/calliope-gui/README.md` shows only that file and `grep -c "Speed training (Loops"
  src/calliope-gui/README.md` prints `1`. Run it once, then stop. Do not commit.
- **routine:** yes

**Order:** Rust 1 → 2 → 3 → 4 → 8 → 5 → 6 → 7 → 9 (2 may run any time before 4; 8 right after 4, because the schema
bump makes the "written as 5" assertions fail until then). Frontend 10 → 11 → 12 → 13 → 14 (10 after 7; 11 any time).
Then 15. 16 any time. Every task leaves `npm test` green.

## 4. Test strategy

- **Pure Rust:** `speed` (steps, ranges, schedule, progress), `player_state` (schema 6, lenient training, speed range,
  Training ↔ Loop On), `tempo_store` (speed kept).
- **Stretcher** (`stretch` tests): pitch on synthetic tones (440 Hz ± 2 Hz at 50-200 %, ± 3 Hz at 25 %), duration
  (± 1 %), alignment of bursts (± 10 ms). Real listening quality is a manual check.
- **Engine** (`player_tests`, synthetic `Pcm`s, `apply`/`render` directly): click-vs-music alignment with the
  two-render method (same timeline, clicks on a silent source vs bursts without clicks), continuity (no 10 ms window
  under 50 % RMS) across speed changes, wraps and passes, position monotonic, count-in spacing, track end, training
  schedule and end rules. 100 % stays bit-exact.
- **Real-time safety:** `tests/player_realtime.rs` counts Rust allocations on the render thread (zero after
  construction). C++ allocations: by code reading (scratch grown once in `Stretcher::new`).
- **Manager/IPC** (`player_tests`, `player_ipc_tests`, ManualBackend, injected `Instant`s): actions, refusals, transport
  fields, logs, the 500 ms write.
- **Acceptance** (`acceptance_player_backend`, ManualBackend): `sp01`-`sp12`, one per criterion (section 7).
- **Frontend:** vitest + `mockIPC` (helpers, state, Speed group, the training popover with its focus and closing rules,
  keys, DOM order); `svelte-check`.
- **GUI:** `gui_player_e2e::player_speed` on a private display (the popover driven by keyboard, its placement checked by
  its `rect` log at 1280x800 and 1024x640), `CALLIOPE_E2E_AUDIO=capture:` WAV analysis (tone
  frequency, click spacing), the layout log, `player.json` on disk, restart; `visual_review` screenshots (never routine).
- **Hardware stand-ins:** no test opens the real audio device (`ManualBackend` in Rust tests, `NullBackend` with capture
  in GUI tests; the static test on `CpalBackend` stays). No network, no archserver, no other host.
- **Whole suite:** `cd src/calliope-gui && npm test`, then `npm run test:gui` (or `DISPLAY=:1 npm run test:gui:serial`).

## 5. Deployment

Laptop only: `cd src/calliope-gui && npm run build:app` and install `target/release/calliope-gui` as before. Nothing
changes on archserver. New build prerequisite: `clang` (`pacman -S clang`; already installed here). This build writes
`player.json` as schema 6; the previous build opens those files read-only (it plays with defaults and never overwrites
them), so don't mix builds on the same repository.

## 6. Manual checks (owner: laptop with real audio)

- **M1: Sound quality.** A real backing track with guitar and drums:
  1. 85 %, 70 %, 50 %: the pitch is the same as at 100 %; strummed chords and drums sound acceptable for practice (some
     softening of attacks below 70 % is normal for this technique);
  2. 120 % and 150 %: same check; 25 % and 200 %: usable, artefacts expected;
  3. no crackles or dropouts over a whole song at 70 % (also with the laptop busy, e.g. a browser open).
- **M2: Changing while playing.** Press `-` and `+` (and `[`/`]`) while playing: the tempo changes within a fraction of a
  second, no gap, the playhead keeps moving forward. Typing a value and Reset do the same.
- **M3: Metronome.** A track with sections and Click on, at 70 %: the clicks stay on the beats of the song, they sound
  as crisp as at 100 % (not stretched), and the readout shows the section BPM × 0.7. Count-in before Play at 70 % is at
  the slower tempo.
- **M4: Loops.** A loop at 70 % repeats without a gap or click at the wrap; Rew/FF, section Start/End and loop Prev/Next
  jump at once.
- **M5: Training.** A 2-bar loop: open "Training…", set 60 → 80, step 10, ×2, switch Training on, close it with Escape
  (and once by clicking outside), loop Play: two passes at 60, two at 70, then
  80 for ever; each change starts at the loop start with no gap; the Loops pane shows "60 % · 1/2" etc.; the speed box
  shows the training speed. Stop: the speed returns to the track speed; Play starts at 60 % again.
- **M6: Saved.** Set 80 %, close the app, reopen the track: 80 %, the training settings are there. An old track opens at
  100 % with its loops and sections as before.
- **M7: CPU.** While playing at 70 %, `top` shows calliope-gui using only a few percent of one core more than at 100 %.

## 7. Acceptance mapping

| # | Criterion | Req | Tasks | Tests |
|---|---|---|---|---|
| SP1 | `-` ×3 from 100 → 85 %, 10 s of song take ~11.8 s | 1 | 2, 5, 7, 13 | speed (`step_pct`), stretch (duration), `sp01`, ControlBar, GUI step 2 |
| SP2 | 440 Hz stays 440 Hz at 50 % and 150 % | 2 | 3, 5 | stretch (pitch), `sp02`, GUI step 2 (tone) |
| SP3 | 100 → 70 % while playing: no pause, position keeps moving from the same place | 3 | 5, 7 | player_tests (crossfade, monotonic), `sp03`, M2 |
| SP4 | 120 BPM at 50 %: shows 60 BPM, clicks every 1 s on the music's beats | 4, 5, 6 | 5, 7, 11, 13 | player_tests (two-render alignment), `sp04`, ControlBar (readout), GUI step 3, M3 |
| SP5 | No tempo map: no BPM shown, speed still slows playback | 6 | 7, 11, 13 | `sp05`, speed-format (`readoutText(0)`), ControlBar |
| SP6 | Typing 20 or 250 is refused, speed unchanged | 1 | 2, 7, 11, 13 | speed (`check_pct`), `sp06`, speed-format, ControlBar, GUI step 4 |
| SP7 | 80 % survives close and reopen | 7 | 4, 7 | player_state, `sp07`, GUI step 8 |
| SP8 | Training 60 → 80, step 10, ×2: 2×60, 2×70, then 80; Loops pane shows speed and repeat | 8, 9, 10 | 2, 6, 7, 11, 14 | speed (schedule), player_tests, `sp08`, ControlBar (popover, status), GUI steps 6-7, M5 |
| SP9 | Stopping returns to the track speed; next Play restarts at 60 % | 11 | 6, 7 | player_tests, `sp09`, GUI step 7 |
| SP10 | Training settings survive close and reopen | 12 | 4, 7, 14 | player_state, `sp10`, GUI step 8 |
| SP11 | A previous-version `player.json` loads at 100 % with loops and sections unchanged | constraint | 4, 8 | player_state (lenient read), `sp11`, M6 |
| (req 13) | Count-in during training at the current training speed | 13 | 5, 6, 7 | player_tests, `sp12` |

## 8. Owner decisions (answered by the owner, 2026-10-18)

Q1: the alternative (the popover); Q2-Q10: the recommended answers. The questions as asked, with the answers:

1. **Layout (spec open question 1). Answer: the alternative.** The training settings go in a small popover opened from
   a "Training…" button in the Loops pane (2.9, 2.11); the progress stays in the Loops status line and the Speed group
   stays in the Playback pane. Originally recommended: the Speed group (`-`, box, `+`, Reset, BPM readout) in the Playback
   pane, wrapping to its second line at 1280x800 (same line on wide windows); training as a 4th line of the Loops pane
   with four compact boxes; the progress in the Loops status line (2.9). Expected: no scrolling at 1280x800 with the
   tree folded, the tablature placeholder shrinks from ~208 px to ~80 px. Alternative: the training settings in a small
   popover from a "Training…" button (no 4th line).
2. **(Answer: as recommended.) Training speeds absolute or relative to the track speed (spec open question 2).** Recommended: absolute, as
   proposed: what is set is what is heard and shown, and it matches the practice apps the spec checked.
3. **(Answer: as recommended.) Which Play starts a training run?** Recommended: only the loop's own Play (Loops pane). Main Play / Space passing
   through a repeating loop repeats it at the track speed, as today. Alternative: any playback that wraps the loop
   trains.
4. **(Answer: as recommended.) Training and Loop On.** Recommended: switching Training on also switches Loop On on, and switching Loop On off
   switches Training off (a run needs repeats). Alternative: independent switches, training only effective with Loop On.
5. **(Answer: as recommended.) The speed control during a run.** Recommended: disabled (also `[`/`]`), showing the training speed, with the reason
   as tooltip. Alternative: changing it ends the run and sets the track speed.
6. **(Answer: as recommended.) Pause during a run.** Recommended: Pause ends the run, like loop Stop (today Pause already ends the loop's Play
   state); Space then plays on at the track speed and the next loop Play restarts at the start %. Alternative: Pause
   keeps the run and Space resumes it at the current pass.
7. **(Answer: as recommended.) `-`/`+` from a typed value off the 5 % grid.** Recommended: snap to the grid (73 → 70 / 75). Alternative: exact ±5
   (73 → 68 / 78).
8. **(Answer: as recommended.) Count-in wording in requirement 13.** It says "only on the first Play, or on every repeat when the loop's Count-In
   is on". The current rule (phase 3, owner decision Q4) is: loop Play counts in only with the loop's own Count-In, and
   then before every pass, including the first. Recommended: keep the current rule unchanged during training (at the
   current training speed). Alternative: a count-in before the first pass of a training run even with the loop's
   Count-In off.
9. **(Answer: as recommended.) Typing a BPM or Autodetect no longer resets the speed** (the old BPM-step design reset it). Recommended: keep the
   speed, since it is now a percentage independent of the BPM. Alternative: reset to 100 % on a new tempo map.
10. **(Answer: as recommended.) Training defaults and the end display.** Recommended: a loop's first training settings are 70 → 100 %, step 5 %, ×2,
    and at the end speed the progress shows the count without a total ("80 % · 3").
