# Plan: gui-section-bpm-detect (Detect BPM of one section from 3 bars at the playhead)

Spec: `specs/gui-section-bpm-detect.md` (owner answers 2026-10-27 in its Open questions section).
Base: `b7e07ee` on `spec/gui-section-bpm-detect`. Paths follow "Paths in this document" of `docs/architecture.md`:
`src/*.rs`, `src/ui/...` and `tests/...` are inside `src/calliope-gui/`.

## 1. Summary

The Player's Sections pane gets a "Detect BPM" button on the Start row, after Find start. It takes up to 3 bars of the
**selected source** from the playhead to the end of the section under the playhead. It encodes that piece to FLAC from
the PCM the Player already holds, and sends it to the existing `POST /v1/beats` of calliope-stems. From the returned
beats it derives one BPM (0.1). It then applies that BPM to the whole section through the same live path as a typed
section BPM (`SectionEdit::Bpm`). The job runs on its own thread, inside the open Player session. Playback goes on. A
section edit, a closed session or another track drops the result. The status shows "BPM old → new, detected from a – b",
plus a ×2 / ÷2 hint for octave errors. No server change, no new dependency, no file-format change.

## 2. Design

### 2.1 What already exists and is reused

- **Beats API and client.** `StemsClient::health` / `supports_beats` / `submit_beats(file, cancel, on_sent)` /
  `wait_final` / `fetch_beats(job, duration_s)` / `delete` (calliope-lib). The server has **no minimum duration**:
  `receive` only checks the FLAC header and the 15-minute maximum, and `check_beats` only bounds the beats by
  `duration + 1 s`. A 1-bar piece of 0.8 s (300 BPM) is accepted. So no server change is needed (spec constraint).
- **The Editor's detection job** (`beat_detect.rs`): health check, part file `.calliope-beats-<uuid>.part` in the track
  folder removed by a `PartGuard`, upload, wait, error mapping (`client_stop`: "Cannot reach the edge-AI server at
  <url>", `STALLED`, 413, server failure text), `NO_BEATS_TASK` for an old server, the server job deleted on every
  exit. Its upload-and-wait part is extracted into a shared helper (§2.4), so both jobs behave the same.
- **The beats-to-tempo maths** (`tempo_detect.rs`): `clean`, `repair` (isolated missed or double beats), `median`,
  `slope` (least-squares period) and `quantise` (folds into 20..300 BPM, rounds to 0.1). One new function `piece_bpm`
  (§2.3) reuses them.
- **The live section edit**: `PlayerManager::edit_sections(id, SectionEdit::Bpm { index, at, bpm }, now)`. It is exactly
  what typing a BPM on a section does (`section-bpm` action → `TempoMap::set_section_bpm`). It changes only that
  segment's BPM and keeps every start. The engine gets the new click grid without a pause (`sync_grid`), loops are
  re-snapped keeping their time, and `player.json` is written 500 ms later (`mark_dirty`, not when read-only). It also
  logs `calliope: player section bpm ...`. Requirements 4 and 6 ("applies live, as a typed BPM does") hold by
  construction.
- **The session's PCM**: `Session.pcm: Arc<Pcm>` is the decoded **selected source** (16-bit interleaved, its own rate
  and channels; non-FLAC sources come from ffmpeg at 44.1 kHz stereo). Sources are sample-aligned, so its frames are
  song time. That is the audio of owner answer 1. No second decode, no ffmpeg, no symphonia.
- **FLAC encoding**: `flacenc` 0.5 (already used by `backing_render`), `flacenc::source::MemSource::from_samples(&[i32],
  channels, 16, rate)`.
- **The event channel**: the Player's `EventSink` (the `PlayerHost` slot that `watch_player` re-attaches after a webview
  reload). So detection events need no new channel. The Editor's job pattern (job thread → shared state → the manager's
  `tick` reaps the outcome) is copied.

### 2.2 The piece (requirement 2, owner answer 2)

`section_bpm::piece(map: &TempoMap, index: usize, playhead_ms: f64, end_ms: f64) -> Result<(f64, f64), String>`:

- `seg = map.segments[index]`, `bar_ms = beats_per_bar · 60000 / seg.bpm`.
- `from = playhead_ms` when the playhead is inside the section (`seg.start_ms <= playhead_ms < section_end`), else
  `seg.start_ms` (owner answer 2026-10-27: the button acts on the section the pane shows and is never disabled because
  the playhead is elsewhere; a section selected on the waveform away from the playhead is measured from its start).
  The lead-in of a first section whose first beat is not set is the same case: the piece starts at the section start,
  and nothing before the section is used.
- `section_end = segments[index + 1].start_ms`, else `end_ms` (the track end).
- `to = min(from + 3 · bar_ms, section_end)`. `PIECE_BARS = 3`.
- If `to − from < bar_ms − 1.0` (1 ms tolerance for float noise), return `Err(TOO_SHORT)`:
  "Less than 1 bar from the playhead to the section's end; move the playhead earlier in the section". This happens
  before any thread or server call.
- Frames: `from_f = round(from · rate / 1000)`, `to_f = min(round(to · rate / 1000), pcm.frames)`. The logged and
  displayed `from_ms` / `to_ms` are those of the frames.

Example (acceptance 1): 100 BPM in 4/4 gives a 2.4 s bar and a 7.2 s piece. With the playhead 3 s before the section
end, the piece is those 3 s (≥ 2.4 s, so it is accepted). At 1 s before the end, it is refused.

### 2.3 One BPM from the beats (requirement 3)

`tempo_detect::piece_bpm(beats_s: &[f64], beats_per_bar: u32) -> Result<PieceBpm { bpm: f64, beats: usize }, String>`:

1. `t = repair(&clean(beats_s))`.
2. If `t.len() < min_piece_beats(beats_per_bar)` (one bar: `beats_per_bar` clamped to 3..=4): return `Err(TOO_FEW)`.
   TOO_FEW = "Too few beats in this piece to measure the BPM; try another place in the section".
3. `d` = the intervals and `m = median(d)`. If any interval is outside `±GLITCH_TOL` (15 %) of `m`: return
   `Err(NO_STEADY)`. NO_STEADY = "No steady beat in this piece; try another place in the section".
4. `bpm = quantise(slope(&t, 0, t.len() − 1)).0`. This is the least-squares period over all beats, folded into 20..300
   BPM and rounded to 0.1, exactly like the sections of Autodetect BPM.

Why not `tempo_detect::detect`: it needs 8 beats, segments, anchors and folds octaves against a neighbour. A 3-bar piece
of one section needs none of that, and the owner wants octave errors shown, not corrected (answer 3).

**Hint (owner answer 3).** `section_bpm::hint(old, new) -> Option<Hint>`, `Hint = Double | Half` (serialised
`"double"` / `"half"`):
- `Double` when `|new/old − 2| ≤ 2·HINT_TOL`;
- `Half` when `|new/old − 0.5| ≤ 0.5·HINT_TOL`;
- with `HINT_TOL = 0.08` (the `OCTAVE_TOL` of `tempo_detect`).

The detected BPM is applied as it is. The hint only changes the status text.

### 2.4 The job (`src/section_bpm.rs`, new) and the shared upload helper

**Shared helper in `beat_detect.rs`** (refactor, Editor behaviour unchanged):
- `pub(crate) enum Stop`, `pub(crate) type R<T>`, `pub(crate) fn client_stop`.
- New `pub(crate) fn check_beats_server(client, url) -> R<()>`: health + `supports_beats`, else `NO_BEATS_TASK`.
- New `pub(crate) fn upload_and_wait(client, url, part, cancel, poll, stall, server_job: &mut Option<String>, on: &mut
  dyn FnMut(DetectPhase, Option<f32>)) -> R<String>` moves the Uploading → (part removed) → Waiting → Detecting →
  final-state check block of `run_inner` verbatim. `on` gets every phase and progress update; the caller's `Reporter`
  decides what is a new phase. It returns the server job id once the job is `Done`.
- `run_inner` then calls these two plus its own render and fetch, keeping its event order and log lines exactly.

**`section_bpm.rs`**:

- **Constants and pure functions.** `PIECE_BARS`, `TOO_SHORT`, `HINT_TOL`, `piece` (§2.2), `hint` (§2.3), and
  `encode_piece(pcm: &Pcm, from_f: u64, to_f: u64) -> Result<Vec<u8>, String>`: the interleaved i16 samples of the
  frames `from_f..to_f`, as i32, 16-bit FLAC at `pcm.rate` with `pcm.channels`.
- **`BpmShared`** (cancel `AtomicBool`, phase `Mutex<DetectPhase>`, finished `AtomicBool`, outcome
  `Mutex<Option<BpmOutcome>>`), the same shape as `DetectShared`. `BpmOutcome = Done { bpm: f64, beats: usize } |
  Failed(String) | Cancelled`. It also has `pub(crate) fn finish(&self, o)`, used by the thread and by tests.
- **`BpmJob`** { `id`, `dir`, `serial`, `view: BpmDetectView`, `pcm: Arc<Pcm>`, `from_f`, `to_f`, `params:
  BpmParams { url, poll, stall }` }.
- **`spawn(job, shared, sink: player::EventSink) -> JoinHandle<()>`** runs these steps on its own thread, with
  `catch_unwind` like `beat_detect::spawn`:
  1. Phase `checking`: `check_beats_server`.
  2. `encode_piece` and write the part file `.calliope-beats-<uuid>.part` in the track folder (`create_new`). A
     `PartGuard` removes it on every path.
  3. `upload_and_wait` with the phases `uploading`, `waiting` and `detecting`.
  4. `fetch_beats(id, (to_f − from_f) / rate)`, then `piece_bpm`. The server job is deleted at the end, as in the Editor.

  Each phase change logs `calliope: player detect id=<id> phase=<p>`. Unless `cancel` is set, it also emits
  `PlayerEvent::BpmDetect { id, detect: view with the phase }`, throttled to `PROGRESS_INTERVAL` within a phase. The
  thread never touches the manager.

### 2.5 The run inside the Player session (`src/player.rs` + child module `src/player_bpm.rs`)

**Where the run lives.**
- `Session` gets `gen: u64` (the open ticket's generation, set in `finish_open`) and `bpm_detect: Option<BpmRun>`.
- `BpmRun { serial, view: BpmDetectView, old_bpm, shared: Arc<BpmShared>, handle: Option<JoinHandle<()>> }` with
  `impl Drop`: sets `shared.cancel`.
- So closing the session, opening another track (or the same one again), the Editor's `release`, and app exit all drop
  the run. The thread then stops at its next cancel check, deletes the server job and leaves its part file removed. No
  result can reach another session. Requirement 6: "another track is opened".
- `PlayerManager` gets a counter `bpm_serial: u64`.

**`player_bpm.rs`** (a child module of `player`, `#[path = "player_bpm.rs"] mod bpm;`, so it sees the private session
fields, like `import_batch` does for the import job), with `impl PlayerManager`:

- **`start_bpm_detect(&mut self, id, index, at, params: BpmParams, now) -> Result<PlayerSnapshot, String>`.** Each
  refusal returns `Err(message)`, logs `calliope: player detect refused id=<id> reason=<r>`, and starts nothing:

  | Check, in order | Message | `reason` |
  |---|---|---|
  | session `id` | `no player session for <id>` (existing text) | (none logged) |
  | tempo empty | "Type a BPM first" | `no-tempo` |
  | `state.read_only` | "This track's Player settings were written by a newer Calliope; they can't be changed" | `read-only` |
  | a run exists | "A BPM detection is already running" | `busy` |
  | `params.url` blank | "Set the edge-AI server in Settings first" (the Editor's text) | `no-server` |
  | `switching.is_some()` | "The source is still loading" | `loading` |
  | `segments[index].start_ms` not within 1e-6 of `at` | "The tempo sections changed; try again" | `stale` |
  | `piece(..)` gives `Err(TOO_SHORT)` | `TOO_SHORT` | `too-short` (owner answer 2: no server call) |

  When every check passes:
  1. `serial = ++bpm_serial`.
  2. `view = { serial, index, at, from_ms, to_ms, source: state.source key, phase: checking }`.
  3. Log `calliope: player detect id=<id> index=<i> source=<key> from_ms=<ms> to_ms=<ms> frames=<n> bpm=<old>` (ms as
     integers).
  4. Spawn the job with `s.pcm.clone()` (an `Arc` clone: no copy under the lock) and the manager's sink.
  5. Store the run and return the snapshot (now carrying `bpm_detect`).
- **`cancel_bpm_detect(&mut self, id) -> Result<PlayerSnapshot, String>`.** It takes the run out of the session (Drop
  sets the cancel flag) and logs `calliope: player detect cancelled id=<id> reason=button`. It returns the snapshot
  without a run. Without a run it just returns the snapshot. The Cancel is therefore immediate: the button is enabled
  again at once and nothing can be applied later. The thread cleans up in the background.
- **`reap_bpm_detect(&mut self, now)`**, called at the end of `tick`. When the session's run is finished, it takes the
  run and reads the outcome:
  - `Done { bpm, beats }`, when the section is unchanged: `segments.get(index)` has `start_ms` within 1e-6 of `at` and
    `bpm == old_bpm`.
    1. Call `self.edit_sections(id, SectionEdit::Bpm { index, at, bpm }, now)`.
    2. On `Ok(snap)`, emit `PlayerEvent::BpmDetected { id, result: { outcome: "applied", serial, index, old_bpm,
       new_bpm: Some(bpm), from_ms, to_ms, hint, message: None }, snapshot: Some(Box::new(snap)) }`.
    3. Log `calliope: player detected id=<id> index=<i> beats=<n> old=<a> new=<b> hint=<double|half|none>
       applied=true`.
    4. On `Err(m)` (for example the section is too short for the new BPM): outcome `failed` with `message: m`.
  - `Done`, when the section was moved, removed or its BPM changed (requirement 6): outcome `dropped` with the message
    "The section changed during the detection; its BPM was left as it was". The log line ends `applied=false
    reason=changed`.
  - `Failed(m)`: outcome `failed` with `message: m`, logged as `calliope: player detect failed id=<id> message=<m>`.
  - `Cancelled`: not emitted (only a cancel or a dropped run sets the flag, and both removed the run already).
- **Snapshot.** `PlayerSnapshot` gains `read_only: bool` (`state.read_only`, for requirement 1's disabled state) and
  `bpm_detect: Option<BpmDetectView>` (the running view with its current phase from `shared`).
- **Events.** `PlayerEvent` gains these variants (tag `kind`, kebab-case):
  - `BpmDetect { id, detect: BpmDetectView }` (`bpm-detect`);
  - `BpmDetected { id, result: BpmResult, snapshot: Option<Box<PlayerSnapshot>> }` (`bpm-detected`).
- **Exit.** `PlayerHost::shutdown` already closes the session, which drops and cancels the run, so it needs no change.
  A test asserts it.

The job never holds the manager lock, and the manager never waits for the thread. Edits, playback and source changes go
on while it runs. A source change after the start does not matter, because the piece's PCM is the `Arc` taken at the
start.

### 2.6 IPC (`src/player_ipc.rs`)

Two new commands, in the Editor's style (`editor_detect` / `editor_cancel_detect`). They need the settings' URL, which
`player_tempo` does not have:
- `player_detect_bpm(id, index, at) -> PlayerSnapshot`: body `do_player_detect_bpm(host, url, id, index, at)` with
  `BpmParams { url, poll: ipc::IMPORT_POLL, stall: STALL_TIMEOUT }`. The URL comes from `SettingsStore` (`edge_ai_url`,
  empty when unset).
- `player_cancel_detect(id) -> PlayerSnapshot`.

Each one is registered in `generate_handler!` (`src/gui.rs`), `COMMANDS` (`build.rs`) and `capabilities/main.json`. The
static test in `tests/frontend.rs` checks the three agree. Tests call the bodies with a 50 ms poll.

### 2.7 Frontend

**`src/ui/lib/ipc.ts`.**
- New types: `DetectPhase = 'checking' | 'uploading' | 'waiting' | 'detecting'`, `BpmDetectView { serial, index, at,
  from_ms, to_ms, source, phase }`, `BpmResult { serial, index, outcome: 'applied' | 'failed' | 'dropped', old_bpm,
  new_bpm: number | null, from_ms, to_ms, hint: 'double' | 'half' | null, message: string | null }`.
- `PlayerSnapshot` gains `read_only: boolean` and `bpm_detect: BpmDetectView | null`.
- `PlayerEvent` gains `{ kind: 'bpm-detect'; id; detect: BpmDetectView }` and `{ kind: 'bpm-detected'; id; result:
  BpmResult; snapshot: PlayerSnapshot | null }`.
- New wrappers: `playerDetectBpm(id, index, at)` and `playerCancelDetect(id)`.

**`src/ui/lib/bpm-format.ts`** (pure text, exact):
- `pieceTime(ms)`: `m:ss.s` (`0:10.5`, `1:02.0`), with tenths rounded.
- `detectStatus(r: BpmResult): string`, with `a = old_bpm.toFixed(1)`, `b = new_bpm.toFixed(1)` and `t = `${pieceTime(from)} – ${pieceTime(to)}``:
  - no hint: `BPM ${a} → ${b}, detected from ${t}` (the spec's example);
  - `double`: `BPM ${a} → ${b} from ${t}, about twice: try ÷2`;
  - `half`: `BPM ${a} → ${b} from ${t}, about half: try ×2`.

  These are short enough (≤ 56 characters for 3-digit BPMs) for the one-line message span. That span has the full text
  as its tooltip.
- `detectRunning(v: BpmDetectView): string`: `Detecting the BPM from ${t}...`, with ` (waiting for the edge-AI
  server)` inserted before the dots when `phase === 'waiting'`.

**`src/ui/lib/player-state.svelte.ts`.**
- `pl.detect: BpmDetectView | null` is set from every snapshot in `applySnapshot` (`s.bpm_detect`) and cleared in
  `resetFields`.
- **Which section.** The button acts on `targetSection()`, the section the pane shows (normally the one under the
  playhead, since the pane follows it). It is never disabled because the playhead is elsewhere (owner answer
  2026-10-27); Rust measures from the section's start then (§2.x `piece`).
- **`detectBpm()`.**
  1. Guards: active, a tempo, not `read_only`, no `pl.detect`, `targetSection()` not null.
  2. Clear `sectionError` / `sectionNote` and log `player detect start index=<i>`.
  3. `await playerDetectBpm(id, i, at)` with generation guard and `applySnapshot`.
  4. On refusal: `pl.sectionError = message` and log `player detect error message=<m>`. The too-short message comes from
     Rust, which owns the rule.
- **`cancelDetect()`.**
  1. Log `player detect cancel`.
  2. `await playerCancelDetect(id)`, then `applySnapshot` (`pl.detect` becomes null).
  3. `pl.sectionNote = 'BPM detection cancelled; the section is unchanged'`.
- **Events in `onEvent`** (already filtered by generation and track id):
  - `bpm-detect`: when `pl.detect?.serial === e.detect.serial`, set `pl.detect = e.detect` and log
    `player detect phase=<p>`. Events of a cancelled run, which no longer match, are ignored.
  - `bpm-detected` with `outcome` `applied`:
    1. `applySnapshot(e.snapshot)`.
    2. Set `pl.sectionError = ''` and `pl.sectionNote = detectStatus(r)`.
    3. Log `player detected index=<i> old=<a> new=<b> hint=<double|half|none>`, then `logSections`.
  - `bpm-detected` with `outcome` `failed`: `pl.detect = null`, `pl.sectionError = r.message`, and log
    `player detect error message=<m>`.
  - `bpm-detected` with `outcome` `dropped`: `pl.detect = null`, `pl.sectionNote = r.message`, and log
    `player detect dropped index=<i>`.
  - Results whose serial does not match `pl.detect` are ignored, except that `applied` still applies its snapshot,
    because the backend state changed.
- Any other section action clears the note, as today (`sectionCall`).

### 2.8 The Start row (`SectionOffset.svelte`, requirements 1, 5, 7, owner answer 4)

**The button.** After Find start:
`<Button variant="outline" class="h-9 w-[5.75rem] px-1 text-sm" data-testid="detect-bpm">`. Its fixed width keeps the
row from moving when the label changes.

- **Idle.**
  - Label "Detect BPM", aria-label "Detect the BPM of the section from 3 bars at the playhead".
  - Disabled, with a `title` giving the first reason that applies:
    - no tempo: "Type a BPM first";
    - `pl.snap.read_only`: "This track's Player settings are read-only";
    - Find start running: "Finding the start...".
  - Click: `detectBpm()`.
- **While `pl.detect` is set** (requirement 5's "offers Cancel"):
  - The same button reads "Cancel", with aria-label "Cancel the BPM detection" and title "Detecting the BPM...". It is
    enabled, and a click runs `cancelDetect()`.
  - The Detect action itself is unavailable while a detection runs. That is requirement 1's "disabled while a
    detection runs", shown by the label change instead of a second greyed button. There is no room for two buttons
    (below).
- **The message line** (`SectionsSection.svelte`) shows `detectRunning(pl.detect)` while running. The priority is:
  finding-start text, then the detect running text, then error, then note.
- **The status line** (row 3, "Section i of n · BPM") already follows the live BPM.

**Width (requirement 7).**
- Row 4 is about 399 px today: "Start" 42 + field 72 + "/64 beat" 68 + time 97 + Find start 84 + 4 gaps × 9. The
  button adds 9 + 103.5. Sections is as wide as its widest row (about 465 px inside, `sections_w=496` outside), and Loops
  wraps below once Sections grows by about 10 px.
- To make room, the unit span becomes `/64` (text-sm, muted, `title="1/64 beat"`; the field's title and aria-label
  already say "1/64 beats"), and the time span becomes `text-sm`. Expected row 4 ≈ 42 + 72 + 25 + 85 + 84 + 103.5 + 5 ×
  9 ≈ 456 px ≤ 465.
- If the measurement says more, apply these fallbacks in order:
  1. Find start `px-1` (−4.5 px);
  2. the detect button `w-[5.25rem]` (−9 px), which still holds "Detect BPM" in text-sm;
  3. drop the visible unit span (−34 px), with the unit kept in the titles.
- No row is added and no height changes, so every window coordinate in the GUI tests stays. The exceptions are
  `FIND_START`, whose x moves left, and the new `SEC_DETECT`. Both are measured in Task 11.
- The check: the `player layout` line reads `overflow=0`, `loops_beside=1` and `sections_w ≤ 496`.

### 2.9 Logs

**Rust (`calliope: `):**
- `player detect id=<id> index=<i> source=<key> from_ms=<ms> to_ms=<ms> frames=<n> bpm=<old>`
- `player detect id=<id> phase=<checking|uploading|waiting|detecting>`
- `player detected id=<id> index=<i> beats=<n> old=<a> new=<b> hint=<double|half|none> applied=<true|false>[ reason=changed]`
- `player detect failed id=<id> message=<m>`
- `player detect cancelled id=<id> reason=<button|closed>` (`closed`: logged by `BpmRun`'s Drop when the run is still
  in the session)
- `player detect refused id=<id> reason=<no-tempo|read-only|busy|no-server|loading|stale|too-short>`
- an applied result also logs the existing `player section bpm id=... index=<i> ...`

**Frontend (`calliope-ui: `):**
- `player detect start index=<i>`
- `player detect phase=<p>`
- `player detected index=<i> old=<a> new=<b> hint=<h>`
- `player detect dropped index=<i>`
- `player detect error message=<m>`
- `player detect cancel`

### 2.10 Test stand-in: the stub tracker (`tests/support/stub-beat-tracker`)

New modes:
- `bpm:<v>`: steady beats `0.25 + k·60/v` below D, every 4th a downbeat. It gives any tempo for any piece length:
  a 3 s piece at 93.4 BPM has 5 beats.
- `slow-bpm:<v>`: 1 s before each progress line, then as `bpm:<v>`. It is used for Cancel and for edits during a run.
- `irregular`: intervals alternating 0.40 s / 0.70 s from 0.25 s. It is "no steady beat". `few` (3 beats) stays "too few
  beats".

**Input copy.** When the mode came from a `stub-tracker-mode` file, the stub copies its input to `stub-tracker-last.flac`
in that file's folder, overwriting it on each run. Tests read the piece that was really sent: its STREAMINFO length and
its samples or tone. The server deletes a beats job's input after the run, so tests cannot read it there. Modes given
by `$STUB_TRACKER_MODE` copy nothing, so the calliope-stems conformance and smoke tests are unchanged.

## 3. Tasks

Every task follows these rules:

- Run only the tests the task touches (named below). The lead runs `npm test` once per task before committing.
- `npm` commands run in `src/calliope-gui/`. Rust is formatted with `cargo fmt --all`.
- Tests never open a real audio device or dialog. They never reach `0.0.0.0:8765` or another host: they use the real
  `calliope-stems` on 127.0.0.1 with the stub tracker (`src/test_server.rs` `TestServer::start_beats`, the acceptance
  file's own starter, or `start_stems_app` in GUI tests). They use temporary repositories only.

### Task 1. Stub tracker: tempo modes and the input copy
- routine: yes
- files: `tests/support/stub-beat-tracker`, `tests/support/README.md`.
- does:
  1. **Modes.** In `read_mode`, return the mode and the folder where the mode file was found:
     `(mode, folder or None)`.
  2. **Validation.** Accept `MODES`, plus `bpm:<v>` and `slow-bpm:<v>` with `v` a float in 10..400 (any other
     `bpm:` text: "unknown mode", exit 64), plus `irregular`.
  3. **`make_beats`.**
     - `bpm:<v>` → `run(d, 0.25, 60.0 / v)`.
     - `irregular` → times from 0.25 s adding 0.40, 0.70, 0.40, ... while below `d`.
     - `slow-bpm:<v>` sleeps 1 s before each progress line like `slow`, then uses `bpm:<v>`.
  4. **Input copy.** After the argv log, when the folder is not None: `shutil.copyfile(inp, os.path.join(folder,
     "stub-tracker-last.flac"))` (stdlib `shutil`), for every mode.
  5. **Docstring and README.** Update the module docstring and the README table: the three modes, and the sentence
     "When the mode comes from a `stub-tracker-mode` file, the input is copied to `stub-tracker-last.flac` next to it
     (each run overwrites it)".
- done when:
  - In a scratch dir, `ffmpeg -nostdin -f lavfi -i sine=frequency=330:sample_rate=22050:duration=7.2 -c:a flac
    in.flac` works.
  - `mkdir -p w/jobs/x && echo bpm:93.4 > w/stub-tracker-mode && (cd w/jobs/x && python3 -I <repo>/src/calliope-gui/tests/support/stub-beat-tracker ../../../in.flac out)`
    writes `w/jobs/x/out/beats.json` with 11 beats spaced 0.642398 s (± 1e-6) and `w/stub-tracker-last.flac`,
    byte-identical to `in.flac`.
  - `STUB_TRACKER_MODE=irregular` gives alternating intervals and creates no copy. `STUB_TRACKER_MODE=bpm:abc` exits
    64.
  - `cargo test -p calliope-stems` passes.

### Task 2. `tempo_detect::piece_bpm`
- routine: no
- files: `src/tempo_detect.rs`, `src/tempo_detect_tests.rs`.
- does: §2.3:
  - `pub(crate) struct PieceBpm { pub bpm: f64, pub beats: usize }`;
  - `pub(crate) fn min_piece_beats(beats_per_bar: u32) -> usize` (clamp 3..=4);
  - `pub(crate) const TOO_FEW`, `pub(crate) const NO_STEADY` with the texts of §2.3;
  - `pub(crate) fn piece_bpm(beats_s: &[f64], beats_per_bar: u32) -> Result<PieceBpm, String>` reusing `clean`, `repair`, `median`, `slope`
    and `quantise`.
- tests:
  - Steady 93.4 BPM: 11 beats from 0.25 s gives 93.4, and so do 5 beats.
  - 4 beats at 120 BPM give 120.0. In 4/4, 3 beats give `TOO_FEW` (in 3/4 they are enough), and an empty list gives
    `TOO_FEW`.
  - Alternating 0.40 / 0.70 s gives `NO_STEADY`.
  - One missed beat inside 12 beats is repaired and the result is the right BPM.
  - Jitter of ±5 ms on 12 beats at 100 BPM gives 100.0 (± 0.1).
  - 400 BPM beats fold to 200.0. 15 BPM beats fold to 30.0.
  - Unsorted and duplicate input is cleaned.
- done when: `cargo test -p calliope-gui --lib tempo_detect` passes; `cargo clippy -p calliope-gui --all-targets -- -D
  warnings` is clean.

### Task 3. Shared upload-and-wait helper in `beat_detect.rs`
- routine: no
- files: `src/beat_detect.rs`.
- does: §2.4 "Shared helper": make `Stop`, `R`, `client_stop` `pub(crate)`; add `check_beats_server` and
  `upload_and_wait` by moving the existing code out of `run_inner`. `run_inner` keeps the same phases, events, log lines
  and error texts in the same order.
- done when: `cargo test -p calliope-gui --lib beat_detect` and `cargo test -p calliope-gui --test acceptance_beat_autodetect`
  pass unchanged; clippy clean.

### Task 4. `section_bpm.rs`: piece, hint, encoding and the job
- routine: no
- files: `src/section_bpm.rs` (new), `src/section_bpm_tests.rs` (new), `src/lib.rs` (`pub mod section_bpm;`).
- does: §2.2, §2.3 hint, §2.4 `section_bpm` (constants, `piece`, `hint`, `encode_piece`, `BpmShared`, `BpmOutcome`,
  `BpmParams`, `BpmJob`, `BpmDetectView`, `spawn`).
  - `BpmDetectView`, the `Hint` serialisation and `DetectPhase` are `Serialize` with the names of §2.1 / §2.7.
  - `PlayerEvent::BpmDetect` is emitted through a `player::EventSink`. To keep this task self-contained, add the
    variant to `PlayerEvent` here (`src/player.rs`, enum only); Task 5 adds the rest.
- tests (`TestServer::start_beats(mode, &[])` from `src/test_server.rs`, 50 ms poll, a `Pcm` built in the test: 22 050
  Hz stereo, left 330 Hz, right a sample counter, so any offset error shows):
  - **`piece`:**
    - 100 BPM, 4/4, section 500..19 000, playhead 10 500 gives (10 500, 17 700).
    - Playhead 16 000 with the next change at 19 000 gives (16 000, 19 000).
    - Playhead 17 500 gives `Err(TOO_SHORT)`, and exactly 1 bar is accepted.
    - Playhead 200 in a first section starting at 500 gives a piece from 500.
    - The last section ends at the track end.
    - 3/4 (`beats_per_bar` 3) gives 3 × 3 beats.
  - **`hint`:** (100, 200.0) → Double; (100, 185) → Double; (100, 183) → None; (100, 50) → Half; (100, 46.5) → Half;
    (100, 93.4) → None.
  - **`encode_piece`:** decoding the bytes with `claxon` gives exactly `pcm.samples[from·ch .. to·ch]`. This holds for
    mono and stereo, at 22 050 and 44 100 Hz.
  - **Job against the server:**
    - `bpm:93.4` gives `Done { bpm: 93.4 }`. The work dir's `stub-tracker-last.flac` has `to − from` frames, and its
      samples equal the slice (the "audio sent" check). No `.calliope-beats-*.part` is left in the track dir.
    - `few` gives `Failed(TOO_FEW)`. `irregular` gives `Failed(NO_STEADY)`. `fail` gives "Beat detection failed on the
      server: ...".
    - A closed port (`http://127.0.0.1:1`) gives "Cannot reach the edge-AI server at ...".
    - A server without a tracker (`TestServer::start("ok")`) gives `NO_BEATS_TASK`.
    - `slow-bpm:120` with the cancel flag set after the `uploading` event gives `Cancelled`, the server job is deleted
      (its status 404), and the part file is gone.
    - The phase events come in the order checking, uploading, waiting/detecting.
- done when: `cargo test -p calliope-gui --lib section_bpm` passes; clippy clean.

### Task 5. The run in the Player session
- routine: no
- files: `src/player.rs`, `src/player_bpm.rs` (new child module), `src/player_tests.rs`.
- does: §2.5. This covers the `Session.gen` and `bpm_detect` fields, `BpmRun` (Drop cancels and logs `reason=closed`),
  `bpm_serial`, `start_bpm_detect`, `cancel_bpm_detect`, `reap_bpm_detect` (called from `tick`), the snapshot fields
  `read_only` and `bpm_detect`, the `BpmDetected` event, and the log lines of §2.9. Add a `#[cfg(test)]` hook
  `test_fake_bpm_detect(&mut self, id, index) -> Arc<BpmShared>` that registers a run without a thread (like
  `EditorManager::test_fake_detect`), so the tests drive the outcome with `shared.finish(..)` and `tick`.
- tests (`player_tests.rs`, ManualBackend, injected time):
  - **Refusals.** Each one checks its message and that `bpm_detect` stays None:
    - no tempo;
    - read-only (newer schema);
    - busy;
    - blank URL;
    - stale `at`;
    - playhead in another section;
    - too short.

    For too-short the URL points to a `TestServer` and its log has no `POST /v1/beats` line.
  - **Snapshot.** A started run shows in the snapshot (`bpm_detect.index`, `from_ms`, `to_ms`, `source`), and
    `read_only` follows the state.
  - **Applied.** A fake run finished with `Done { bpm: 93.4 }`, then `tick`:
    - the section's BPM is 93.4, every `start_ms` and the other BPMs are unchanged;
    - the `BpmDetected` event has `outcome: applied`, the hint and the snapshot;
    - after 500 ms and `tick`, `player.json` holds 93.4.
  - **Hint.** Old 100, new 200 gives `hint: double`.
  - **Dropped.** Before the finish: the section moved (`SectionEdit::Move`), the section removed, or its BPM typed
    (`SectionEdit::Bpm`). Each gives `outcome: dropped`, with the map exactly as the edit left it.
  - **Session gone.** `close()`, or opening another track, sets the run's cancel flag, and a later finish emits
    nothing.
  - **Cancel.** `cancel_bpm_detect` leaves the BPM unchanged, sets the flag and clears `bpm_detect`. A later
    `finish(Done)` + `tick` changes nothing and emits nothing.
  - **Failed.** `Failed("x")` gives `outcome: failed`, `message: "x"`, and the map is unchanged.
  - **While playing, Click on.** Play with ManualBackend and pull 1 s. Finish `Done { bpm: 120 }` on a 100 BPM section,
    `tick`, pull 2 s:
    - the transport stays playing;
    - the click onsets before the edit are 600 ms apart and after the next beat 500 ms apart (the existing onset helper
      of the `ts*` tests);
    - the source samples are continuous across the edit.
- done when: `cargo test -p calliope-gui --lib player` passes (the existing player tests too); clippy clean.

### Task 6. IPC commands and registration
- routine: no
- files: `src/player_ipc.rs`, `src/player_ipc_tests.rs`, `src/gui.rs`, `build.rs`, `capabilities/main.json`.
- does: §2.6, the two command bodies and Tauri wrappers, and the three registrations. The settings URL is read in the
  wrapper, as in `editor_detect`.
- tests (`player_ipc_tests.rs`, `env()` + `TestServer::start_beats`, poll 50 ms; the helper ticks `host.tick` every 20 ms
  until a `bpm-detected` event, at most 10 s):
  - **Track 304** (backing 330 Hz, vocals 440, guitar 660; 22 050 Hz), with `player.json` rewritten to
    `{"start_ms":500,"bpm":100,"found_ms":500}, {"start_ms":17800,"bpm":120,"detected_ms":17800}`:
    - **Playhead 10 500, `bpm:93.4`.**
      - The copy has 158 760 frames (± 1).
      - Its samples equal the decoded backing slice.
      - The section becomes 93.4.
      - After `do_close_player` (flush), `player.json` has 93.4 at 500 and 120 at 17 800.
    - **Selected source `stem:guitar`** (`do_player_select_source`): the copy equals the guitar slice, not the backing.
      This is the spec's "audio" acceptance, with the source selected in the dropdown.
    - **Playhead 14 800:** the copy has 3.0 s of frames.
    - **Playhead 17 000:** too short, and the server log has no new `POST /v1/beats`.
  - **Shutdown.** `PlayerHost::shutdown` during a `slow-bpm` run cancels it, and the server job is gone.
  - **Registration.** `tests/frontend.rs` sees the new commands in all three places.
- done when: `cargo test -p calliope-gui --lib player_ipc` and `cargo test -p calliope-gui --test frontend` pass;
  clippy clean.

### Task 7. Detect status texts
- routine: yes
- files: `src/ui/lib/bpm-format.ts`, `src/ui/lib/bpm-format.test.ts`.
- does: add exactly as §2.7 `bpm-format.ts`, with JSDoc. Use local structural types, so the task does not depend on
  `ipc.ts`:
  - `pieceTime(ms: number): string`:
    1. `const t = Math.round(Math.max(0, ms) / 100) / 10; const m = Math.floor(t / 60); const s = (t - m * 60).toFixed(1);`
    2. Return `` `${m}:${Number(s) < 10 ? '0' : ''}${s}` ``.
  - `detectStatus(r: { old_bpm: number; new_bpm: number | null; from_ms: number; to_ms: number; hint: 'double' | 'half' | null }): string`
    with the three texts of §2.7. A null `new_bpm` is treated as `old_bpm`.
  - `detectRunning(v: { from_ms: number; to_ms: number; phase: string }): string` with the text of §2.7.
- tests:
  - `pieceTime`: 10500 → `0:10.5`; 17700 → `0:17.7`; 62000 → `1:02.0`; 59960 → `1:00.0`; 0 → `0:00.0`.
  - `detectStatus`: `{92, 93.4, 41200, 47100, null}` → `BPM 92.0 → 93.4, detected from 0:41.2 – 0:47.1`;
    `{100, 198.4, 10500, 17700, 'double'}` → `BPM 100.0 → 198.4 from 0:10.5 – 0:17.7, about twice: try ÷2`;
    `'half'` → `... about half: try ×2`.
  - `detectRunning`: `checking` → `Detecting the BPM from 0:10.5 – 0:17.7...`; `waiting` → `Detecting the BPM from 0:10.5
    – 0:17.7 (waiting for the edge-AI server)...`.
- done when: `npx vitest run src/ui/lib/bpm-format.test.ts` passes; `npm run check` is clean.

### Task 8. IPC types and player state
- routine: no
- files:
  - `src/ui/lib/ipc.ts`;
  - `src/ui/lib/player-state.svelte.ts`;
  - `src/ui/lib/player-state.test.ts`;
  - the snapshot builders (one line each: `read_only: false, bpm_detect: null`) in
    `src/ui/components/player/ControlBar.test.ts`, `src/ui/acceptance_player.test.ts` and `src/ui/views/PlayerView.test.ts`.
- does: §2.7 `ipc.ts` and `player-state.svelte.ts` (`pl.detect`, `detectBpm`, `cancelDetect`, the
  event handling, `applySnapshot`, `resetFields`, the logs of §2.9).
- tests (`player-state.test.ts`, mockIPC):
  - **Start.** `detectBpm()` with `tempo_index: 1` sends `player_detect_bpm` with `{id, index: 1, at: <start of 1>}`. The
    reply's `bpm_detect` sets `pl.detect`.
  - **Phases.** A `bpm-detect` event with the same serial updates the phase. One with another serial is ignored.
  - **Applied.** The `applied` result applies its snapshot (the new BPM), sets `sectionNote` to the `detectStatus` text
    and clears `pl.detect`.
  - **Failed.** It sets `sectionError` and changes no segments.
  - **Dropped.** It sets the note to the message.
  - **Refusal.** A refused start (reject with the too-short text) sets `sectionError`, and `pl.detect` stays null.
  - **Cancel.** `cancelDetect()` sends `player_cancel_detect`, clears `pl.detect` and sets the cancel note. A late
    `bpm-detect` event of that serial is ignored.
  - **Guards.** No call without a tempo, when `read_only`, or while `pl.detect` is set. After a track switch an event
    of the old track or generation is ignored.
- done when: `npx vitest run src/ui/lib/player-state.test.ts src/ui/components/player src/ui/views src/ui/acceptance_player.test.ts`
  passes; `npm run check` is clean.

### Task 9. The Detect BPM button on the Start row
- routine: no
- files: `src/ui/components/player/SectionOffset.svelte`, `src/ui/components/player/SectionsSection.svelte`,
  `src/ui/components/player/ControlBar.test.ts` (its "Sections group" describe).
- does: §2.8 (the button, the Cancel label while running, the titles, the `/64` unit and `text-sm` time, and the message
  line priority).
- tests (mockIPC):
  - **States.**
    - The button reads "Detect BPM" after Find start in the Start row (`data-testid="detect-bpm"`).
    - It is disabled with "Type a BPM first" without a tempo, and with the read-only title when `read_only`.
    - With the target selected in section 2 while `tempo_index` is 0, it is enabled and sends `index: 2`.
  - **Running.** A click sends the command. While `bpm_detect` is set:
    - the button reads "Cancel" and is enabled;
    - the message line shows "Detecting the BPM from 0:10.5 – 0:17.7...";
    - a click sends `player_cancel_detect`, after which the button reads "Detect BPM" and is enabled again.
  - **Messages.** An `applied` event shows the status text in `section-message` and the new BPM in `section-status`. A
    `failed` event shows the message with role alert.
  - The existing Start-row tests still pass, with the unit now `/64`.
- done when: `npx vitest run src/ui/components/player src/ui/lib/player-state.test.ts` passes; `npm run check` is clean;
  `npm run test:ui` passes.

### Task 10. Headless acceptance tests
- routine: no
- files: `tests/acceptance_section_bpm.rs` (new).
- does:
  - Use the pub API only: `PlayerManager` with `ManualBackend`, `begin_open`/`load`/`finish_open` or `open`,
    `start_bpm_detect`, `cancel_bpm_detect`, `edit_sections`, `tick`, `select_source` path.
  - The real `calliope-stems` on 127.0.0.1:0 with the stub tracker. The starter is a copy of `real_server` from
    `acceptance_beat_autodetect.rs`, which also collects the stderr lines and exposes the work dir for
    `stub-tracker-last.flac`.
  - The library-player fixture is copied to a temp dir, with track 304's `player.json` as in Task 6. Poll 50 ms.
  - One test per acceptance criterion of the spec (names `ac01_..` to `ac10_..`), as mapped in §7.
- done when: `cargo test -p calliope-gui --test acceptance_section_bpm` passes; clippy clean.

### Task 11. Player GUI test and new positions
- routine: no
- files: `tests/common/mod.rs` (helper), `tests/gui_player_e2e.rs`.
- does:
  1. **Helper.** `start_player_stems(dirs, tracker_mode) -> (App, String)` in `tests/common/mod.rs`:
     - like `start_detect_app`, but `start_stems_app(dirs, "ok", tracker_mode, false, &[], Some(dirs.capture()))` on
       `player_dirs` dirs, then Alt+5 and `wait_view("player")`;
     - it creates `stems-work/` and `home/`;
     - the app and calliope-stems run inside `unshare -rn` on 127.0.0.1:8765, never the real server.
  2. **Measure** `FIND_START` (moved left) and the new `SEC_DETECT` at 1280x800 with the tree folded, from a screenshot or
     with `xdotool getmouselocation`. Every other constant stays (no height change; check with one `player_first_beat`
     run).
  3. **New test `player_section_bpm_detect`** (`#[test] #[ignore]`):
     - Dirs: `player_dirs` with track 304's `player.json` = schema 7
       `{"start_ms":500,"bpm":100,"found_ms":500,"metronome":true}, {"start_ms":17800,"bpm":120,"detected_ms":17800,"metronome":false}`,
       source `backing:backing`.
     - Start with `start_player_stems(&dirs, "bpm:93.4")`, then `pick` "practice" (304).
     - The steps:
     1. **Layout.** The last `player layout` line has `overflow=0`, `loops_beside=1` and `sections_w ≤ 496`. Take
        screenshot `player-detect.png`.
     2. **7.2 s from the playhead.** Enter `0:10.5` in `TIME` and click `SEC_DETECT`.
        - Expect `calliope: player detect id=... index=0 source=backing:backing from_ms=10500 to_ms=17700`.
        - Then `player detected ... new=93.4 hint=none applied=true`, and the frontend `player detected index=0 old=100
          new=93.4`.
        - `wait_json`: segment 0 `{start_ms 500, bpm 93.4, found_ms 500}`, segment 1 `{17800, 120}`.
        - `stems-work/stub-tracker-last.flac` lasts 7.2 s (± 1 frame, via `calliope_lib` STREAMINFO or `ffprobe`), and
          its 330 Hz power is at least 20 dB above its 660 Hz power (`decode` + `tone_power`).
        - No `.calliope-beats-*.part` is in the track folder.
        - Take screenshot `player-detect-done.png` (the status "BPM 100.0 → 93.4, detected from 0:10.5 – 0:17.7").
     3. **Stops at the section end.** Enter `0:14.8`, then Detect. Expect `from_ms=14800 to_ms=17800`, and the copy lasts
        3.0 s.
     4. **Too short.** Enter `0:17.0`, then Detect.
        - Expect `player detect refused ... reason=too-short` and the frontend `player detect error`.
        - The stems log has no new `request method=POST path=/v1/beats` line (count before / after).
        - `player.json` is unchanged after 500 ms.
        - Take screenshot `player-detect-short.png`.
     5. **Selected source, ×2 hint.**
        1. `set_tracker_mode(&dirs, "bpm:186.8")`, choose the Guitar source (`choose_source`; the 304 order is Backing,
           Vocals, Guitar), enter `0:10.5`, then Detect.
        2. Expect `source=stem:guitar`, `hint=double`, and segment 0 bpm 186.8. The copy's 660 Hz power is at least
           20 dB above its 330 Hz power.
        3. Take screenshot `player-detect-double.png` ("..., about twice: try ÷2").
        4. Press `SEC_HALVE`: `wait_json` bpm 93.4.
     6. **No steady beat.** `set_tracker_mode(.., "irregular")`, then Detect. Expect `player detect failed ...
        message=No steady beat`. Segment 0 stays 93.4 after 1 s. Take screenshot `player-detect-failed.png`.
     7. **Cancel.** `set_tracker_mode(.., "slow-bpm:150")`, then Detect.
        - Wait for `phase=uploading` (or later) and take screenshot `player-detect-running.png` (the button reads Cancel,
          the message line says "Detecting the BPM ...").
        - Click `SEC_DETECT` (Cancel). Expect `player detect cancelled ... reason=button`.
        - After 4 s (longer than the slow run), there is no `player detected` line and segment 0 is 93.4.
        - Take screenshot `player-detect-cancelled.png` (the button reads Detect BPM, enabled).
     8. **Removed during the run.** Still `slow-bpm:150`. Enter `0:17.9` (section 2, 2.1 s ≥ 1 bar at 120), then Detect.
        After `phase=uploading`, click `SEC_REMOVE`.
        - Expect the `player section remove` line, then `player detected ... applied=false reason=changed`.
        - `wait_json`: one segment, bpm 93.4.
     9. **Playing with the Click on.**
        1. `set_tracker_mode(.., "bpm:120")`. Enter `0:02.0` and press Play. After 1 s, Detect. Wait for `player
           detected ... new=120 ... applied=true`, then 2 s more, then pause.
        2. No `calliope: player transport playing=false` line between Play and the pause.
        3. In the capture since the Play mark, the click onsets (`click_onsets_ms`) before the `applied` time are
           642 ms ± 3 apart (the section is at 93.4 BPM after step 5). After it, from the next beat, they are 500 ms
           ± 3 apart.
        4. The 330 Hz tone continues (no 10 ms window below 50 % RMS around the edit).
     10. **Another track opened during a run.** `slow-bpm:150`, enter `0:10.5`, Detect. After `phase=uploading`, `pick`
         another track (301).
         - Expect `player detect cancelled ... reason=closed`.
         - Pick 304 again after 4 s: its `player.json` segment 0 is still 120 (the value of step 9).
     11. **Layout again.** Repeat the layout assertion.
  4. **Existing tests.** `player_first_beat` and `player_first_beat_tuning` use the moved `FIND_START`. Their layout
     assertions stay.
- done when:
  - `npm run test:gui -- player_section_bpm_detect player_first_beat` passes (or `DISPLAY=:1 npm run test:gui:serial --`
    with the same names).
  - The whole `gui_player_e2e` binary passes: `npm run test:gui -- --test gui_player_e2e` (or serial).
  - `CALLIOPE_GUI_SHOTS=1` `visual_review` was run once, and its 1280x800 assertion passes.
  - The screenshots were looked at: the Start row is on one line with "Detect BPM" fully readable after Find start, with
    nothing clipped. The button reads "Cancel" while running. The status texts are readable in the message line (the
    double hint included). Loops is beside Sections.
  - `cargo fmt --all --check` is clean.

### Task 12. Docs: README and UI guide
- routine: no (edits inside two long files; content checked by reading)
- files: `src/calliope-gui/README.md`, `docs/ui.md` ("Decided by the team" and the Sections group bullet).
- does:
  - **README**, Sections paragraph: "Detect BPM (on the Start row) sends up to 3 bars from the playhead to the end of
    the section under it, from the source selected in the dropdown, to the edge-AI server and sets the section's BPM to
    the result (playback goes on; Cancel stops it; an edit of the section meanwhile drops the result). Less than 1 bar
    is refused. When the result is about twice or half the old BPM, the status suggests /2 or x2." The README is ASCII
    only, so it uses "/2" and "x2" and a plain arrow `->` if one is needed.
  - **`docs/ui.md`.**
    - Line 183 ("... no global Count In and no Detect button"): the Player now has a section Detect BPM on the Start
      row (the whole-track detection stays in the Editor).
    - Update the fourth-row description: "Start [field] /64 time [Find start] [Detect BPM]". The unit and the time are
      `text-sm`, and the button is `w-[5.75rem]`; it reads Cancel while running.
    - Add the message-line priority (finding start, then detecting, then error, then note).
    - Update the measured layout numbers and the `FIND_START` / `SEC_DETECT` positions from Task 11.
    - Append a `(gui-section-bpm-detect)` bullet with:
      - the disabled titles;
      - the running text;
      - the three status formats;
      - the Cancel note;
      - the dropped message.
- done when: `git diff --stat` shows only those two files; `LC_ALL=C grep -nP '[^\x00-\x7F]' src/calliope-gui/README.md`
  prints nothing new; `npm test` still passes.

## 4. Test strategy

- **Pure logic** (Rust unit tests):
  - `tempo_detect_tests.rs` `piece_bpm`: steady, too few, irregular, repaired, jitter, folding.
  - `section_bpm_tests.rs`: `piece` (the 7.2 s / 3 s / < 1 bar rules, the lead-in, 3/4), `hint`, `encode_piece`
    round trip with claxon (bit-exact slice).
  - Vitest `bpm-format.test.ts`: the status and running texts.
- **Job against the server**:
  - The stand-in is the real `calliope-stems` binary on `127.0.0.1:0` (`TestServer::start_beats`) with
    `tests/support/stub-beat-tracker`, never Beat This!.
  - Its modes `bpm:<v>`, `slow-bpm:<v>`, `irregular`, `few`, `fail` and `hang`, plus a closed port and a server without
    a tracker, stand for every server answer.
  - The piece that was really uploaded is checked through the stub's `stub-tracker-last.flac` (length and samples).
- **Manager**: `player_tests.rs` (ManualBackend, a fake run driven by `BpmShared::finish`) covers:
  - the refusals and the guards: moved, removed, typed, closed, another track, cancel;
  - the live application: clicks follow, the source is continuous, the 500 ms write.
- **IPC bodies**: `player_ipc_tests.rs` runs the whole command path on the library-player fixture with the real server
  and a 50 ms poll. This includes the selected source and `shutdown`.
- **Headless acceptance**: `tests/acceptance_section_bpm.rs`, one test per acceptance criterion except the layout one.
- **Frontend state and components** (vitest + mockIPC): `player-state.test.ts` and `ControlBar.test.ts`.
- **GUI on a display**: `gui_player_e2e::player_section_bpm_detect` (§3 Task 11) runs inside `unshare -rn` with
  calliope-stems on 127.0.0.1:8765 and the capturing audio backend. It checks:
  - the log lines;
  - `player.json`;
  - the uploaded copy's length and tone;
  - the capture WAV's click spacing;
  - the screenshots (looked at by the agent);
  - the `player layout` line.
- **Waits** stay short:
  - a 50 ms poll in Rust tests;
  - the app's 1 s poll in the GUI;
  - `slow-bpm` is about 2 s;
  - the GUI server's beat timeout is 3 s.
- **Whole headless suite**: `cd src/calliope-gui && npm test`. **GUI suite**: `cd src/calliope-gui && npm run test:gui`
  (or `DISPLAY=:1 npm run test:gui:serial`).

## 5. Deployment

Only calliope-gui changes. It runs on the laptop (`npm run build:app`, then `target/release/calliope-gui`). The
calliope-stems server on archserver is **not** changed: the feature uses its existing `POST /v1/beats`, so it needs no
redeploy. The stub tracker is test-only and is never deployed.

## 6. Manual checks (owner, on the laptop against the real server)

1. **Basic detection.**
   1. Make sure Settings has the edge-AI server and the server lists the beats task (Autodetect BPM works in the
      Editor).
   2. Open a track whose section BPM is off. Put the playhead a few bars into the section, where the playing is steady
      (drums in).
   3. Press "Detect BPM" on the Start row. The message line should say "Detecting the BPM from a – b..." and the button
      should read "Cancel".
   4. After a few seconds the section's BPM label and the status should change. The message should read "BPM old → new,
      detected from a – b". The range should be about 3 bars from the playhead.
2. **Listen.** Turn the section's Click on and play the section. The clicks should now sit on the beat across the
   section, not only where you detected.
3. **While playing.** Press Detect BPM during playback. Playback should not stop, and the clicks should switch to the
   new tempo when the result arrives.
4. **Source.** Choose a stem in the dropdown (for example the drums, if the track has them) and detect again. The result
   comes from that stem. Compare it with the backing.
5. **Near the end.** Put the playhead less than a bar before the next section, then Detect. A "Less than 1 bar ..."
   message should appear at once.
6. **Octave.** If a result is about twice or half the old BPM, the message should end with "try ÷2" / "try ×2", and the
   ÷2 / ×2 buttons should fix it.
7. **Cancel.** Start a detection while the server is busy (for example during a stem extraction) and press Cancel. The
   section should keep its BPM, and the button should read Detect BPM again.
8. **Server down.** Stop calliope-stems on archserver (or unplug the LAN) and press Detect. You should see "Cannot reach
   the edge-AI server at ...", and nothing should change.
9. **Saved.** Reopen the track (or restart the app). The detected BPM should still be there.
10. **Layout.** At 1280x800 with the tree folded, the Start row should be on one line, and Loops should still be beside
    Sections.

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests |
|---|---|---|
| 100 BPM 4/4, playhead 10 s into the section: the 7.2 s from the playhead are sent | 1, 4, 5, 6, 10, 11 | `section_bpm_tests` `piece` (10 500..17 700) and the job's copy = slice; `player_ipc_tests` (158 760 frames, backing slice); `acceptance_section_bpm` `ac01`; GUI step 2 (`from_ms=10500 to_ms=17700`, copy 7.2 s) |
| Playhead 3 s before the section's end: those 3 s only | 4, 6, 10, 11 | `piece` (16 000..19 000); `player_ipc_tests` (14 800 → 3.0 s); `ac02`; GUI step 3 |
| Less than 1 bar before the end: message, server not called | 4, 5, 6, 8, 10, 11 | `piece` `Err(TOO_SHORT)`; `player_tests` refusal with no `POST /v1/beats` in the server log; `player_ipc_tests`; `player-state.test.ts` (sectionError); `ac03`; GUI step 4 (refused line, POST count unchanged, screenshot) |
| "audio" source selected: the piece comes from it, not the backing | 4, 5, 6, 10, 11 | `player_ipc_tests` (guitar slice); `ac04`; GUI step 5 (`source=stem:guitar`, 660 Hz copy) |
| Tracker gives 93.4 BPM: section 93.4, its start and other sections unchanged, `player.json` holds it | 1, 2, 4, 5, 6, 10, 11 | `piece_bpm` tests; `player_tests` applied (starts unchanged, 500 ms write); `player_ipc_tests`; `ac05`; GUI step 2 (`wait_json`) |
| Success: status shows old and new BPM and the range; about twice: points to ×2 / ÷2 | 4, 5, 7, 8, 9, 11 | `hint` tests; `bpm-format.test.ts` texts; `player-state.test.ts` note; `ControlBar.test.ts` message line; `ac06` (result fields); GUI steps 2 and 5 screenshots |
| Server down or no steady beat: message, BPM kept | 2, 4, 5, 8, 10, 11 | `section_bpm_tests` (closed port, `irregular`, `few`, `fail`, no tracker); `player_tests` failed outcome; `player-state.test.ts`; `ac07`; GUI step 6 |
| Cancel during a run: BPM kept, button enabled again | 4, 5, 8, 9, 10, 11 | `section_bpm_tests` cancel; `player_tests` cancel + late finish ignored; `player-state.test.ts`; `ControlBar.test.ts`; `ac08`; GUI step 7 |
| Section removed or another track opened during a run: result not applied | 5, 6, 8, 10, 11 | `player_tests` dropped (move/remove/typed) and session gone; `player_ipc_tests` shutdown; `player-state.test.ts` generation guard; `ac09`; GUI steps 8 and 10 |
| Playing with the Click on: clicks follow the new BPM, playback goes on | 5, 10, 11 | `player_tests` (ManualBackend onsets 600 → 500 ms, continuous source); `ac10`; GUI step 9 (capture onsets, no `playing=false`) |
| Player at 1280x800: no overflow, Loops beside Sections | 9, 11 | GUI steps 1 and 11, `player_first_beat*` layout assertions, `visual_review` (`overflow=0`, `loops_beside=1`, `sections_w ≤ 496`) |

## 8. Open questions

None. The owner accepted the team decisions below (2026-10-27), changing 1:

1. **The section acted on** (owner answer 2026-10-27). The button acts on the section the pane shows, which follows
   the playhead; it is never disabled because the playhead is elsewhere. When a section selected on the waveform is
   away from the playhead, the piece starts at that section's start.
2. **"Disabled while a detection runs".** The Detect BPM button turns into Cancel in the same place while it runs. There
   is no room on the Start row for a greyed Detect BPM next to a Cancel.
3. **Lead-in.** With the first beat not set and the playhead before the first section's start, the piece starts at the
   section's start.
4. **"Edited meanwhile".** Only the section itself counts: its start moved, its BPM changed or the section removed. Edits
   of other sections, a source change or seeking during the run do not drop the result. Opening the same track again
   also drops it, because the session it started in is gone.
5. **Start row unit.** The visible unit next to the Start field becomes `/64` (it was `/64 beat`) to make room. The field's
   tooltip and label keep "1/64 beats".
