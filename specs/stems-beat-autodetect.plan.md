# Plan: Stems beat autodetection (Editor mix waveform, Autodetect BPM, tempo sections)

Revision 2 (final). Base `f03dcfd`, branch `spec/stems-beat-autodetect`. Builds on:
- `specs/gui-backing-track-editor.plan.md`: Editor engine, Mix lane;
- `specs/gui-play-backing-track.plan.md`: tempo map 2.3, `player.json` 2.5, waveform 2.10, detection sketch 2.13;
- `specs/gui-stem-extraction.plan.md`: calliope-stems protocol, separator process model, stub separator.

The owner's answers to revision 1's questions are applied (section 8). Changes from revision 1:
- **Step and limit:** changes move in 1/64-beat steps, at most ±4 beats from the detected position in total.
- **Shortest section:** 1 bar.
- **Player session:** closed (not updated live) when the Editor writes the tempo.
- **Player waveform:** shows the sections.
- **Editor tabs:** the "BPM & sections" tab is removed.

## 1. Summary

**Mix waveform.**
- The Editor's Mix lane gets the waveform view the Player has, drawn from the current mix (the checked stems at
  their volumes).
- It is recomputed in Rust, on a worker thread, 0.3 s after the last change. A new change restarts the wait.

**Autodetect BPM.**
- The button renders the mix and sends it to `calliope-stems` on archserver. There, "Beat This!" (CPJKU, MIT code and
  weights) finds the beats and downbeats.
- It is reached through a new additive `/v1/beats` endpoint and a "beat tracker" adapter command.
- The app turns the beats into tempo sections, as short as one bar. Each change sits on the 1/64-beat grid of the
  section before it, and its detected position is remembered.

**Where the result goes.**
- The sections become the track's tempo map in `player.json` (schema 2), the map the Player's metronome and count-in
  use.
- If the Player has the track open, its session is flushed and closed first. It reloads the next time it shows the
  track.

**Display and fine adjustment.**
- Both waveforms show the start BPM at the left edge, and an amber bar plus the new BPM at each change.
- In the Editor, a detected change can be dragged or moved with the keys in 1/64-beat steps of the left section's
  tempo, at most 4 beats from its detected position. In the Player the sections are read-only.

**Removed:** the Player's disabled "Detect" button and the Editor's "BPM & sections" placeholder tab.

## 2. Design

### 2.1 What is reused, what is new

Reused unchanged:
- `player_state` load/save, `apply_tempo_change` (loops re-snapped, keeping their time);
- `backing_render::render_flac`;
- `mixer::{gain_factor, to_stereo_f32, mix}`;
- the calliope-stems job queue, the process runner and the stub pattern;
- `stems_client` (health, `wait_final`, `delete`);
- the editor channel/event pattern;
- `bpm-format.ts`, `waveform-geometry.ts`.

New or changed:

| Where | What | Pure? |
|---|---|---|
| `src/calliope-lib/src/stems_api.rs` | `Health.tasks`, `JobStatus.kind` (additive), `Beats {beats, downbeats, tracker}`, `check_beats(&Beats, duration_s)` | yes |
| `src/calliope-lib/src/stems_client.rs` | `supports_beats(&Health)`, `submit_beats(file, cancel, on_sent)`, `fetch_beats(job, duration_s)` | client |
| `src/calliope-stems/src/{cli,config,jobs,server}.rs` | `--beat-tracker PATH`, `--beat-timeout-min`, hidden `--beat-timeout-ms`; job kind `beats`; `POST /v1/beats`, `GET /v1/jobs/<id>/beats`; `tasks` in health | server |
| `src/calliope-stems/trackers/beat-this.sh` | real tracker adapter (bash + embedded Python), runs Beat This! from its own venv | archserver |
| `src/calliope-gui/tests/support/stub-beat-tracker` | test stand-in (Python 3 stdlib) | test |
| `src/calliope-gui/src/tempo_map.rs` | `Segment.detected_ms`; tick bases and the stretched last beat for detected changes; `set_change_offset`; anchor rules in `set_bpm_at` | yes |
| `src/calliope-gui/src/player_state.rs` | `player.json` schema 2 (segments may carry `detected_ms`); reads 1 and 2 | yes |
| `src/calliope-gui/src/tempo_detect.rs` | beats + downbeats → `TempoMap` (optimal segmentation, sections ≥ 1 bar, 1/64-beat snapping) | yes |
| `src/calliope-gui/src/mix_waveform.rs` | `mix_peaks(...)`, `Debounce` (0.3 s restartable deadline) | yes |
| `src/calliope-gui/src/tempo_store.rs` | `trait TempoStore { load, replace }`, `FileTempoStore` (`player.json`) | yes (file IO) |
| `src/calliope-gui/src/beat_detect.rs` | the detection job | job thread |
| `src/calliope-gui/src/editor.rs` | peaks scheduling/worker, session `tempo`, detection start/cancel/outcome, events | manager |
| `src/calliope-gui/src/player.rs`, `player_ipc.rs` | `PlayerManager::release(id)` (flush + close + `PlayerEvent::Closed`), `finish_open` re-reads a changed `player.json`; `PlayerTempoStore` | Player internals |
| `src/calliope-gui/src/ipc.rs`, `gui.rs`, `build.rs`, `capabilities/main.json` | `editor_detect`, `editor_cancel_detect`, `editor_tempo`; exit cancels detection | Tauri glue |
| `src/ui/components/player/WaveformView.svelte` | props-driven, shared; sections (read-only unless `onmove`), drag, keys | component |
| `src/ui/lib/waveform-geometry.ts` | `fineMs`, `sectionMarks`, `labelFits`, `dragSteps` | yes |
| `src/ui/lib/editor-state.svelte.ts`, `components/editor/MixLane.svelte`, `views/EditorView.svelte` | Mix lane waveform, Autodetect BPM, progress/Cancel, messages, tempo reload | frontend |
| `src/ui/lib/player-state.svelte.ts`, `components/player/{PlayerPane,BpmSection}.svelte`, `views/PlayerView.svelte` | `closed` event → reopen; sections in the waveform; Detect removed | frontend |
| `src/ui/lib/views.ts` | "BPM & sections" tab removed | frontend |

### 2.2 Where the tempo map lives (owner: `player.json`)

**One tempo map per track: `tempo` in `player.json`, shared by the Editor and the Player.**
- **Why here:** the metronome and the count-in read it, and all sources of a track are sample-aligned.
- **"Save the information along the mix track"** means the map is written to `player.json` in the track folder
  within 1 s.
- **Detected positions are stored per change** (2.3). The raw beat list is not stored.
- **Replacing the map uses the existing Player rule** (`apply_tempo_change`):
  - loops keep their time (their stored ticks are recomputed for the new map, exactly as when a BPM is typed);
  - `speed` resets to 1.0;
  - `beats_per_bar` (4) and every other field are kept;
  - the playback position is never touched.
- **A detection replaces the whole map without a dialog** (owner). The result line says "Detected 2 tempos: 120, 90
  BPM (replaced the previous tempo map)". There is no undo.
- **A newer `player.json`** (schema > 2) is never overwritten. Detection and moves fail with "player.json was
  written by a newer version; the tempo was not saved".

**`TempoStore`, and the Player session** (owner 8a: close it, don't update it live):
- **`replace(id, dir, f)`** runs entirely under the `PlayerHost` lock:
  1. `PlayerManager::release(id)`. If the Player has a session for this track:
     - it flushes the pending state (loops, position, source, metronome) to `player.json`. If that write fails, the
       replace stops with "Could not save the Player's settings first: …" and nothing else changes;
     - it stops playback and closes the output;
     - it drops the session and emits `PlayerEvent::Closed {id}`.
  2. `FileTempoStore`: load `player.json` (now the Player's latest state) → `f(old)` → `apply_tempo_change` →
     `speed = 1.0` → `write_atomic`. The track folder is never created.
- **`load(id, dir)`** also runs under the lock. If the Player holds the track with unsaved changes, it flushes them
  first (the session stays open), then reads the file.
- **No lost writes, Editor → Player:** the Editor's write starts from the flushed file and changes only `tempo`
  (plus the loops' ticks re-expressed at the same times, and `speed`). The Player's loops and position are therefore
  never lost.
- **No lost writes, Player → Editor:** after the release the Player has no session for the track, so no debounced
  Player write can follow and overwrite the new tempo.
- **A Player open that read `player.json` before the write:**
  - The Player loads the file off-lock and installs the session in `finish_open` under the lock.
  - `Loaded` now carries the FNV-1a revision of the bytes it read.
  - `finish_open` re-reads `player.json`. If the revision changed, it re-parses the state under the lock (a small
    file).
  - So a write that lands between the read and the install is never lost.
- **The Player UI** reacts to `closed`: player-state forgets the session (`key = null`) and bumps `pl.reopen`.
  - `PlayerView`'s selection effect depends on `pl.reopen`. If the Player view is shown, it reopens the track at once
    (loading, then paused at the saved position). If it is not shown, it reopens the next time it is.
  - A playback that was running in the Player stops: the tempo changed under it. That can only happen when a
    detection finishes after the user switched to the Player and pressed Play.
- **Freshness in the Editor:** the Editor reads the map through `load` when it opens a track and on every Editor
  view entry (`editor_tempo` `reload`), so BPMs typed in the Player appear.

### 2.3 Tempo-change resolution: 1/64 beat for changes, the 1/64-note grid unchanged for loops (owner 4)

Owner: the move step is **1/64 of a beat**, so 4 beats are 256 steps.

**Constraint.** The Player's grid is a tick = 1/64 note = 1/16 beat (`TICKS_PER_BEAT = 16`), and loops are stored as
ticks in `player.json`.

**Chosen approach: tempo-change positions are stored in milliseconds, which they already are.** A segment is
`{start_ms, bpm}`. Only *detected* changes get the finer resolution and two new rules. The tick grid, typed BPMs and
loop snapping stay as they are.

- **Fine step.** `FINE_STEPS_PER_BEAT = 64`, so `fine_ms(bpm) = 60000 / (bpm * 64)` (7.8125 ms at 120 BPM). A
  detected change is placed, and later moved, in fine steps of its **left** section's tempo.
- **New optional segment field `detected_ms`** (stored in `player.json`): the position detection gave the change,
  already snapped to the left section's fine grid.
  - A segment with `detected_ms` is a **detected change**; one without it is a **typed change** (from the Player's
    BPM field), or the first segment.
  - The first segment never carries it: it is the anchor (bar 1, beat 1), has no bar and can't be moved.
- **Tick bases.** Typed changes keep today's rule, `B_i = B_{i-1} + round(offset in ticks of segment i-1)`, so
  existing maps and loops are unchanged.
  - For a detected change the base is rounded to the nearest whole beat on the global count:
    `B_i = 16 * round((B_{i-1} + (start_i - start_{i-1}) / tick_ms(bpm_{i-1})) / 16)`.
  - The new section's first beat is therefore exactly at `start_i` (a tempo change happens on a beat: "BPM changes
    only can occur when previous beat is done"), and bars keep counting.
- **Stretched last beat.** The ticks `[a, B_i)` with `a = max(B_i - 16, B_{i-1})` (the last beat of the left
  section) are spread linearly between `ms_at_grid(a)` and `start_i`.
  - The fine offset, at most ±1/2 beat after the whole-beat rounding, is absorbed by that one beat, which lasts
    between 1/2 and 3/2 beats.
  - `ms_at` and `tick_near` stay strictly monotonic and inverse to each other.
  - The metronome's last click before a change sits where that stretched beat begins, and the next click is on the
    change.
- **The minimum distance** keeps this valid. A change stays at least 1 beat (of the left tempo) after the previous
  change, and at least 1 beat (of its own tempo) before the next one, so `B_i - B_{i-1} ≥ 16`.
- **Not chosen:**
  - raising `TICKS_PER_BEAT` to 64: it would change how every loop snaps (to 1/256 notes) and need a loop-tick
    migration;
  - a second "fine tick" grid next to the tick grid: two grids to keep consistent.

  Storing the positions in ms needs neither.

**Player behaviour.**
- Loop snapping is **unchanged**: loops still snap to the nearest 1/64 note.
- Typed BPMs, the anchor rule and every existing map behave as before.
- The only new behaviour is inside a detected map: in the last beat before a detected change, the 16 ticks are
  spread over that beat's actual (stretched) length.

**`player.json` becomes schema 2**, the existing newer-schema mechanism:
- **This build** reads 1 and 2, writes 2, and keeps `detected_ms` in segments; a schema-3 file is read-only.
- **Older builds** show a schema-2 file as "written by a newer version" and never overwrite it, so an old build can
  never silently drop the detected positions or re-round the bases.
- **Existing v1 files** have no detected changes, so their meaning is identical.

**Moving a change (owner 5: at most ±4 beats from the detected position, in total).**
- `TempoMap::set_change_offset(index, steps) -> Result<(TempoMap, i64 /* applied */), String>`:
  - **Refused** when `index == 0`, when the segment has no `detected_ms` ("Only detected tempo changes can be moved;
    type the BPM again in the Player to change this one"), and when `|steps| > MAX_OFFSET_STEPS = 256` (a frontend
    bug, not hidden).
  - **The new start** is `detected_ms + steps * fine_ms(bpm_{index-1})`, with the left section's current BPM.
  - **Clamped** to the 1-beat distances above. It returns the applied steps.
  - **Later changes keep their `start_ms`**, so they stay with the audio. Their bases are recomputed, and loops keep
    their time (`apply_tempo_change`).
- **Step semantics:**
  - The total limit holds because the offset is always measured from `detected_ms`: a drag or a key press sends the
    absolute target offset, not a delta.
  - The current offset shown is `round((start_ms - detected_ms) / fine_ms(bpm_{index-1}))`.
  - If the left tempo was changed after detection, the steps use the new left tempo; the limit is still 4 beats of
    it.

**What happens to the stored detected positions** (owner 5):

| Event | Effect on `detected_ms` |
|---|---|
| Autodetect again | the whole map, anchors included, is replaced by the new detection |
| Moving a change in the Editor | `start_ms` changes; `detected_ms` stays |
| Typing a BPM in the Player while at a change's start (`tick_near(position) == B_i`) | the bpm is replaced; `start_ms` and `detected_ms` stay (only the tempo is corrected) |
| Typing a BPM elsewhere | a typed change is inserted (no anchor); detected changes keep theirs (the left section of a later detected change may now be the typed one, whose tempo then sets that change's step) |
| Typing 0 at a position | the segment in effect there is removed with its anchor; if the first segment is removed, the new first segment drops its `detected_ms` |
| Equal neighbouring BPMs merging (existing `dedup`) | the later segment, and its anchor, disappear |

### 2.4 What is analysed (owner: the current Editor mix)

- **What:** the checked stems at their volumes, with solo ignored. This is the mix Save would write (`render_flac`
  re-reads the stems at full precision).
- **Where it goes:** rendered to `<track>/.calliope-beats-<uuid>.part`, a dot name the scans ignore (like
  `.calliope-backing-*.part`). It is removed after the upload, on failure, on cancel and on exit.
- **Format and size:** stereo at the stems' rate, ≤ about 100 MB for 15 minutes.
- **Too long:** a mix over `MAX_DURATION_S` (900 s) is refused before rendering, with "Tracks longer than 15
  minutes can't be analysed".

### 2.5 Detection: Beat This! on archserver (owner: accepted, including the training-data note)

- **Model:** Beat This! 1.1.0, checkpoint `final0`.
- **Licence:** the code and the published weights are MIT (README: "The code and the published model weights are
  released under the MIT license"; PyPI `License-Expression: MIT`). Some training data is copyrighted or under
  limited CC licences; the owner accepted this for personal use.
- **Rejected alternatives:**
  - a local non-AI detector (weaker on real music and tempo changes);
  - madmom (non-commercial models);
  - aubio and Essentia (GPL/AGPL);
  - librosa (one tempo only).

**Runtime on archserver** (the owner installs; agents never do):
- A **separate** uv venv `~/edge-ai/beats/.venv` (Python 3.12) with `torch==2.11.0` and `torchaudio==2.11.0` from
  the cu128 index, then `beat-this==1.1.0` and `soundfile`. An upgrade there can't break Demucs.
- The checkpoint is downloaded once to `~/edge-ai/beats/models/final0.ckpt` (about 78 MB).
- The adapter uses CUDA, falling back to the CPU when CUDA is unavailable or runs out of memory (Ollama may hold most
  of the card).

**Protocol additions** ("calliope-stems API v1", additive):
- **`GET /v1/health`** gains `"tasks": ["stems"]`, or `["stems","beats"]` with a tracker (absent = stems only).
  - An app that finds no `beats` says: "The edge-AI server cannot detect beats yet; update calliope-stems on the
    server (see its README)".
- **`POST /v1/beats`**: a FLAC body, with the same checks as `POST /v1/jobs` (411, 413, 415, 503).
  - `501 {"error":"beat detection is not configured on this server"}` without a tracker.
  - It returns `202 {"job","state":"queued"}`.
  - Beat jobs share the single queue and worker with separation jobs (one GPU).
- **`GET /v1/jobs/<id>`**: a beats job's status has `"kind":"beats"` and `"stems": null`. Stem jobs have no `kind`
  key, so their JSON is unchanged.
- **`GET /v1/jobs/<id>/beats`**: `200 {"beats":[s..],"downbeats":[s..],"tracker":"…"}`, `409` if the job is not done,
  `404` if it is unknown or not a beats job.
  - `stems/<n>` of a beats job → 404.
- **`DELETE /v1/jobs/<id>`** cancels either kind.

**Beats document** (`stems_api::check_beats`, used on the server and in the client):
- seconds; finite; `0 ≤ t ≤ duration_s + 1`; strictly increasing;
- ≤ 20 000 beats;
- every downbeat equals a beat within 1e-6 s;
- `tracker` ≤ 100 characters;
- JSON ≤ 1 MiB (the client's general 64 KiB cap doesn't apply to this endpoint).

**Tracker contract:**
- It is called as `<tracker> <input.flac> <out_dir>` in the job folder.
- It writes `<out_dir>/beats.json` and exits 0.
- It may print `progress <0..1>` lines on stdout.
- **Failures:** any other exit status, a bad or missing file, or a timeout fails the job, with the message (and the
  stderr tail) going to the client.
- **Timeout:** `--beat-timeout-min` defaults to 10; the hidden `--beat-timeout-ms` (minimum 100) is for tests.
- The process group is killed on cancel, timeout and server exit.
- No `stem_levels` are measured for beat jobs.
- **Log lines:** `job id=… kind=beats state=…` and `job id=… beats=<n> downbeats=<m> tracker="…"`.

**Real adapter** `src/calliope-stems/trackers/beat-this.sh`:
- **Environment:** `BEATS_HOME` (default `~/edge-ai/beats`), `PY=$BEATS_HOME/.venv/bin/python`, and the checkpoint
  `$BEATS_HOME/models/final0.ckpt` (override with `BEAT_THIS_CHECKPOINT`).
- **What it does:** it checks its arguments, then runs `"$PY" - "$@" <<'PY'`. The embedded program:
  - reads the FLAC with `soundfile` and mixes it down to mono;
  - prints `progress 0.1`;
  - runs `beat_this.inference.Audio2Beats(checkpoint_path, device, dbn=False)`;
  - prints `progress 0.9`;
  - writes the JSON atomically (tmp + rename).
- **Tests:** syntax only (`bash -n`, plus `ast.parse` of the extracted heredoc with `python3 -I`). No test runs it.

### 2.6 From beats to tempo sections (`tempo_detect.rs`, owner 6: sections as short as 1 bar)

`detect(beats_s, downbeats_s, beats_per_bar) -> Result<Detected { map: TempoMap, sections: Vec<SectionInfo> },
String>`, pure.

**Why the earlier rule had to change.** Revision 1 required a change to last 8 beats, judged by a median over 8
intervals. A 1-bar section has only 4 intervals and is a minority in any 8-interval window, so that rule can never
see it. Revision 2 replaces it with an exact optimal segmentation that allows 4-beat sections.

Constants:
- `MIN_BEATS = 8`;
- `MIN_SECTION_BEATS = 4` (one 4/4 bar: 4 intervals);
- `CHANGE = 0.04` (4 %);
- `PENALTY = 30` (see step 3);
- `MAX_SECTIONS = 64`.

1. **Clean the input.**
   - Drop non-finite and negative values, sort, and drop duplicates within 1 ms.
   - Fewer than `MIN_BEATS` beats → `Err("No steady beat was found in this mix; check more stems (the drums help)
     and try again")`.
   - Repair **isolated** tracker glitches only. An interval is a glitch when it is ≈ k × the median of the 3
     intervals on each side (k = 2 or 3, within 15 %: a missed beat, so k-1 interpolated beats are inserted), or
     ≈ 0.5 × (an extra beat, which is removed). In both cases those 6 neighbours must agree within 15 %.
   - A genuine section of at least one bar at another tempo (≥ 4 deviating intervals) is never "repaired".
2. **The cost of a candidate section** of beats `a..b` (`b - a ≥ MIN_SECTION_BEATS`; neighbouring sections share
   the change beat) is the sum of squared residuals of a least-squares line, beat time against beat index. Prefix
   sums make it O(1).
3. **Optimal partitioning:** minimise `Σ cost + P · (#sections)` by dynamic programming with PELT pruning (O(n)
   typical, O(n²) worst case; n ≤ 4 500 for 15 minutes at 300 BPM).
   - `P = PENALTY · σ²`, where `σ = max(5 ms, 1.4826 · median|IBI_i − IBI_{i−1}| / √2)` is the tracker's jitter (IBI
     = the interval between two beats).
   - The tests of Task 8 pin the behaviour. The implementer may tune `PENALTY` within 10..100 so that all of them
     pass, and must record the final value in the code and the task log.
4. **Sensitivity rule** (the 4 % rule, re-checked for 1 bar): neighbouring sections whose fitted BPMs differ by
   ≤ `CHANGE` are merged and refitted, the closest pair first, until none is left.
   - **So a change is reported when the tempo differs by more than 4 % for at least one bar.** For a 1-bar section,
     the 4-interval fit has to stand out from the jitter.
   - A 1-bar section 4-5 % off is found when the jitter is ≤ about 10 ms (typical for a clean beat track). Larger
     changes, such as half-time or double-time endings, are found at any realistic jitter.
   - Single fills and missed beats don't create sections (step 1, plus `P`).
   - Above `MAX_SECTIONS`, the closest pairs are merged.
5. **BPM per section:**
   - `bpm = round(600 / T) / 10`, from the fitted period T;
   - folded into 20..300 by doubling or halving;
   - neighbouring sections with the same rounded BPM are merged.
6. **Anchor (bar 1, beat 1):**
   - the first downbeat in section 0, or its first beat if there is none;
   - refined by the intercept of a fixed-slope fit (slope = the quantised period) over section 0, moved to that
     beat. This centres the drift from the 0.1-BPM rounding (≤ about ±60 ms over 4 minutes).
   - The first segment is `{start_ms: anchor, bpm_0}`, with no `detected_ms`.
7. **Changes.** For section k ≥ 1:
   - take the fixed-slope intercept at its first beat, `t`;
   - snap it to the left section's fine grid anchored at the left section's start:
     `start = start_{k-1} + round((t - start_{k-1}) / fine_ms(bpm_{k-1})) * fine_ms(bpm_{k-1})` (spec req. 4 at
     1/64-beat resolution);
   - push `{start_ms: start, bpm_k, detected_ms: start}`. A change less than 1 beat after the previous one is merged.
8. **Bars:** `beats_per_bar` keeps the stored value (4). The tracker's most frequent bar length is only logged
   (`bar_beats=<n>`).

Log line: `calliope: editor detected id=… beats=<n> downbeats=<m> bar_beats=<b> sigma_ms=<s> segments=<k> bpm=<..>
starts_ms=<..>`.

### 2.7 The mix waveform and its debounce

- **What it shows:** the mix of the checked stems at their volumes (`mixer::gain_factor`), solo ignored,
  hard-clipped as in playback.
- **How:** `mix_waveform::mix_peaks(stems: &[StemPcm], gains, total_frames, buckets = 2048, cancel) ->
  Option<Vec<f32>>` mixes 4096-frame chunks with `to_stereo_f32` + `mix` and keeps the max |sample| per bucket.
  - The buckets follow the rule of `player_source::peaks`.
  - Cancel is checked between chunks.
  - All gains 0 gives zeros at once.
- **The debounce, in Rust:** `Debounce { due }`, `DELAY = 300 ms`.
  - `touch(now)` sets `due = now + 300 ms`, so a new change restarts the wait.
  - `take_due(now)` fires once.
  - `EditorManager::set_stem(.., now)` calls `touch`. At open the first computation is due at once.
- **Non-blocking:** when `tick(now)` finds it due, it cancels a running computation, bumps `peaks_gen` and spawns a
  worker thread.
  - The worker gets the `Arc` of the in-memory stems and a copy of the gains, and never takes the manager lock.
  - Its result goes into a shared slot. The next tick installs it if its generation is still current and emits
    `EditorEvent::Peaks`.
  - Session close and track switch cancel it.
  - A test hook (`set_peaks_fn`, `#[doc(hidden)] pub`) replaces the computation.
- **Display:** the snapshot carries `peaks`, and the UI keeps the old peaks until new ones arrive.
- **Log:** `calliope: editor peaks id=… gen=<n> ms=<ms>`.

### 2.8 The detection job (`beat_detect.rs`)

`EditorManager::start_detect(id, DetectParams { url, store: Arc<dyn TempoStore>, poll, stall })`.

**Refusals:**
- "Check a stem first": no checked stem above Off (owner 7);
- "Beat detection is already running": one detection per app;
- "Set the edge-AI server in Settings first": no URL.

**Phases** (`EditorEvent::Detect {id, phase, progress}`, ≤ 10 per second):
1. `checking`: health; the server must list the `beats` task.
2. `mixing`: `render_flac` into the part file.
3. `uploading`: `submit_beats`; the part file is removed right after.
4. `waiting`: the job is queued on the server.
5. `detecting`: the job is running.
6. `applying`: `fetch_beats` → `tempo_detect::detect` → `store.replace` (which releases the Player session, 2.2).

**Polling:** `ipc::IMPORT_POLL` (1 s) in the app, 50 ms in tests; `stall` = `STALL_TIMEOUT`.

**Outcome:** the job puts `Ok(TempoMap)`, `Err(message)` or "cancelled" into `DetectShared`. The manager's `tick`
emits exactly one final event:
- `Detected {id, tempo, beats, replaced}`, after updating `session.tempo`;
- `DetectFailed {id, message}`;
- `DetectCancelled {id}`.

**Cancel:** `cancel_detect`, a track switch, `close` and app exit (2 s cap). The client stops and deletes the server
job (best effort), and the part file is removed. A cancel during `applying` is too late: the job ends `Detected`.

**Messages** (inline, in the Mix lane's reserved slot):
- unreachable: "Cannot reach the edge-AI server at <url>";
- no beats task: as in 2.5;
- 413: "The server refused the mix: <msg>";
- tracker failure: "Beat detection failed on the server: <msg>";
- stall: "The edge-AI server stopped answering; try again";
- too few beats: 2.6;
- newer `player.json`: 2.2;
- Player flush failure: 2.2;
- success: "Detected N tempos: a, b BPM" (+ " (replaced the previous tempo map)" when one existed), or "Detected 1
  tempo: 120 BPM".

### 2.9 Behaviour and layout

**Editor Mix lane, top to bottom:**
1. The waveform: full width, about 4 rem; the visual review sets the final height.
2. Mix cell, Play/Pause, Stop, and **Autodetect BPM** right-aligned. While a detection runs the button reads
   **Cancel**, and the phase text shows ("Preparing the mix 40%", "Uploading 70%", "Waiting for the edge-AI
   server", "Detecting beats", "Saving the tempo").
3. Position slider and time field.
4. Save and "Save as".
5. The reserved message slot (the newest of the save and detection messages).

Nothing moves when state changes.

**Autodetect BPM is enabled** when the pane is active, at least one stem is checked with its volume above Off, the
edge-AI URL is set, and no detection runs.
- Hint without a URL: "Set the edge-AI server in Settings to detect the BPM".
- Hint without a stem: "Check a stem to detect its BPM".

**The waveform drawing** (one component, 2.10):
- peaks in `--muted-foreground`, and a 1 px `--foreground` playhead;
- the start BPM label at the left edge (no bar);
- a 2 px amber (`--primary`) full-height bar at each change, with the new BPM right of it;
- labels sit at the bottom of the lane, clear of the loop marks' top bar in the Player. A label reads "120 BPM" if
  it fits before the next bar, else "120", else it is omitted;
- the `aria-label` lists every tempo ("… tempo 120 BPM from the start, 90 BPM from 0:06.5");
- BPM 0: no labels. No checked stem: a flat line.
- In the Editor, typed changes (no `detected_ms`) are drawn at 55 % opacity and can't be moved (their tooltip
  explains why). In the Player every bar is drawn the same.

**Editor drag (detected changes only):**
- Pointer-down within 8 px of a bar starts it (`ew-resize`). The preview uses `dragSteps`: the pointer position,
  rounded to whole fine steps of the left tempo from `detected_ms`, clamped to ±256 and to the 1-beat neighbour
  limits (the same formula as Rust).
- A readout next to the waveform shows "+19/64 beat (+0:00.148) · limit ±4 beats".
- Pointer-up sends `editor_tempo(id, {kind:'move', index, steps})` once, with the absolute offset. Escape cancels.
- At full-track zoom a pixel is many fine steps (about 20 at 120 BPM over 4 minutes), so the drag is coarse and the
  keys give exact steps.

**Editor keys:**
- The canvas is focusable (`tabindex=0`, `role="slider"`, `aria-valuetext` "Tempo change 1 of 2 at 0:06.508, 90
  BPM, +1/64 beat") when the map has at least one detected change.
- Focus selects the first detected change (drawn brighter and thicker). `↑`/`↓` move the selection between detected
  changes.
- `←`/`→` = ∓/± 1 step (1/64 beat); `Shift+←`/`Shift+→` = ∓/± 64 steps (1 beat). Each press sends the new absolute
  offset, clamped to ±256 (coalesced: one call in flight, the latest target wins).
- Space keeps its own meaning there (`[role="slider"]` is in the Editor's Space exclusions).

**Tempo freshness:** `editor_tempo(id, {kind:'reload'})` on each Editor view entry.

**Editor tabs:** "BPM & sections" is removed (owner). Stems, Tablature and MIDI cues remain.

**Player:**
- The "Detect" button is removed.
- The waveform shows the sections, read-only, below the loop marks.
- `closed` → reopen (2.2).
- Nothing else changes.

### 2.10 Frontend modules

| File | Change |
|---|---|
| `src/ui/lib/waveform-geometry.ts` | `fineMs(bpm)`, `sectionMarks(segments, durationMs, width) -> {x: number \| null, bpm, index, detected: boolean}[]`, `labelFits(x, nextX, textWidth)`, `dragSteps(detectedMs, pointerMs, leftBpm, minMs, maxMs) -> number` (rounded, clamped to ±256 and the neighbour limits), `offsetSteps(startMs, detectedMs, leftBpm)`; pure, tested |
| `src/ui/components/player/WaveformView.svelte` | props `peaks`, `durationMs`, `positionMs`, `loops?`, `currentLoop?`, `segments?`, `onmove?(index, steps)`, `testid`, `label`; interaction only with `onmove` |
| `src/ui/components/player/PlayerPane.svelte` | passes `pl.snap` data including `segments` (read-only) |
| `src/ui/components/player/BpmSection.svelte` | Detect removed |
| `src/ui/lib/player-state.svelte.ts`, `src/ui/views/PlayerView.svelte` | `closed` event → `key = null`, `pl.reopen++`; the view's selection effect depends on `pl.reopen` |
| `src/ui/lib/editor-state.svelte.ts` | `ed.peaks`, `ed.tempo`, `ed.detect`, `ed.detectMessage`, `ed.detectError`; `detect()`, `cancelDetect()`, `moveChange(index, steps)` (coalesced), `reloadTempo()`; `canDetect()`; events `peaks`, `detect`, `detected`, `detect-failed`, `detect-cancelled`; log lines |
| `src/ui/lib/ipc.ts` | wrappers and types of 2.11 |
| `src/ui/components/editor/MixLane.svelte`, `src/ui/views/EditorView.svelte` | waveform row, Autodetect BPM/Cancel, phase text, messages; `reloadTempo()` on mount |
| `src/ui/lib/views.ts` | the `tempo` entry of `TRACK_TABS` removed |

### 2.11 IPC

```ts
interface TempoSegment { start_ms: number; bpm: number; detected_ms?: number | null }
interface TempoSnap { beats_per_bar: number; segments: TempoSegment[]; multi: boolean }   // Player's SnapTempo gains detected_ms too
interface DetectState { phase: 'checking'|'mixing'|'uploading'|'waiting'|'detecting'|'applying'; progress: number | null }
interface EditorSnapshot { /* existing */ peaks: number[]; tempo: TempoSnap; detect: DetectState | null }
type EditorEvent = /* existing */
  | { kind: 'peaks'; id: string; peaks: number[] }
  | ({ kind: 'detect'; id: string } & DetectState)
  | { kind: 'detected'; id: string; tempo: TempoSnap; beats: number; replaced: boolean }
  | { kind: 'detect-failed'; id: string; message: string }
  | { kind: 'detect-cancelled'; id: string };
type TempoEdit = { kind: 'move'; index: number; steps: number /* absolute offset from detected_ms, -256..256 */ } | { kind: 'reload' };
type PlayerEvent = /* existing */ | { kind: 'closed'; id: string };
```

| Command | Returns | Notes |
|---|---|---|
| `editor_detect(id)` | `EditorSnapshot` | async; reads the URL from settings; starts the job |
| `editor_cancel_detect(id)` | `()` | |
| `editor_tempo(id, action: TempoEdit)` | `EditorSnapshot` | `move`: `set_change_offset` through `TempoStore::replace`; `reload`: through `TempoStore::load` |

Three new commands: in `generate_handler!`, `COMMANDS` in `build.rs` and `capabilities/main.json`. `editor_set_stem`
passes `Instant::now()`.

**Log lines:**
- Frontend (`calliope-ui: `):
  - `editor waveform peaks=<count> max=<0..1>`
  - `editor detect start|cancel`
  - `editor detect phase=<p>`
  - `editor detected segments=<k> bpm=<a,b,..>`
  - `editor detect error message=<m>`
  - `editor tempo move index=<i> steps=<s> applied=<s'>`
  - `editor tempo reload segments=<k>`
  - `player reopen id=<id>`
- Rust (`calliope: `):
  - `editor peaks id=<id> gen=<n> ms=<ms>`
  - `editor detect id=<id> phase=<p>`
  - `editor detected …` (2.6)
  - `editor detect failed id=<id> message=<m>`
  - `editor detect cancelled id=<id>`
  - `player released id=<id> flushed=<b>`
  - `tempo saved id=<id> segments=<k> detected=<n>`

## 3. Tasks

Rust tasks end with `cargo clippy --workspace --all-targets -- -D warnings` clean; frontend tasks end with `npm run
check` clean (both in `src/calliope-gui/`). One GUI test file is run with:

```
cd src/calliope-gui && npm run build && cargo build -p calliope-stems && \
DISPLAY=:1 cargo test -p calliope-gui --features e2e-hooks --test <file> -- --ignored --test-threads=1
```

(or `bash scripts/test-gui.sh -- <test-name-filter>` on private displays).

### Task 1: Beats types, validation and client (calliope-lib)
- **files:** `src/calliope-lib/src/stems_api.rs`, `src/calliope-lib/src/stems_client.rs`
- **does:**
  - **Types:**
    - `Health.tasks: Vec<String>` (`#[serde(default)]`);
    - `JobStatus.kind: Option<String>` (`skip_serializing_if` none);
    - `Beats {beats, downbeats, #[serde(default)] tracker}`;
    - `MAX_BEATS = 20_000`, `MAX_BEATS_JSON_BYTES = 1 << 20`.
  - **`check_beats`** (2.5).
  - **Client:**
    - `supports_beats`;
    - `submit_beats` (shares a private upload function with `submit`; `POST /v1/beats` without a model query);
    - `fetch_beats(job, duration_s)` (1 MiB cap, `check_beats`; 409 → `Invalid("not done")`, 404 → `Restarted`).
  - **Tests:** every `check_beats` rule at and over its boundary; old health JSON without `tasks`; a stem status
    serialises without `kind`.
- **done when:** `cargo test -p calliope-lib --all-features` passes.
- **routine:** no

### Task 2: Stub beat tracker
- **files:** `src/calliope-gui/tests/support/stub-beat-tracker` (new, executable, `#!/usr/bin/env python3`, stdlib
  only), `src/calliope-gui/tests/support/README.md`
- **does:**
  - **Contract:** `stub-beat-tracker <input.flac> <out_dir>`. It reads the duration D from STREAMINFO and writes
    `<out_dir>/beats.json`. The mode comes from `$STUB_TRACKER_MODE`, else the first line of `stub-tracker-mode` in
    the current directory or two levels up, else `ok`.
  - **Modes:**
    - `ok`: 120 BPM from 0.5 s for the most whole 4-beat bars that end at or before D/2, then 90 BPM from the next
      120-BPM beat time to D. For the 16 s "Six Lanes" stems: 12 beats at 0.5..6.0 s, then 15 beats at
      6.5..15.83 s.
    - `steady`: 120 BPM from 0.5 s to D.
    - `bar-end`: 120 BPM from 0.5 s, with the last full bar before D at 60 BPM (a 1-bar half-time ending).
    - `few`: 3 beats.
    - `fail`: stderr message, exit 3.
    - `slow`: 1 s before each progress line.
    - `hang`.
    - `bad-output`: writes `not json`.
  - Downbeats are every 4th beat from the first. It prints `progress 0.5` and `progress 1`. `$STUB_TRACKER_LOG` gets
    the argv appended.
  - **README:** a section in the style of the stub separator's.
- **done when:** `STUB_TRACKER_MODE=ok tests/support/stub-beat-tracker tests/fixtures/import/stems/drums.flac <tmp>`
  writes 27 beats (12 + 15) and 7 downbeats (checked by Task 4's conformance test). `bash -c 'STUB_TRACKER_MODE=fail
  tests/support/stub-beat-tracker x y; test $? -eq 3'` succeeds.
- **routine:** no

### Task 3: calliope-stems: beat jobs, routes, flags
- **files:** `src/calliope-stems/src/{cli,config,jobs,server,main}.rs`
- **does:**
  - **Flags:** `--beat-tracker PATH` (optional, an executable file), `--beat-timeout-min` (10), hidden
    `--beat-timeout-ms` (minimum 100, wins over minutes in any order); `cli` unit tests.
  - **Jobs:**
    - `Job.kind` (`Stems | Beats`) and `Job.beats`;
    - the worker runs the tracker (argv `[input.flac, out_dir]`, same supervision, the beat timeout, progress, stderr
      tail);
    - on success it reads `out/beats.json` (≤ 1 MiB) and runs `check_beats` with the upload's duration;
    - no `measure_stems`; the status shows `kind`.
  - **Server:**
    - `POST /v1/beats` (shared upload path; 501 without a tracker);
    - `GET /v1/jobs/<id>/beats` (200/409/404); `stems` on a beats job → 404;
    - `tasks` in health;
    - `route_table` tests (`/v1/beats/`, `/v1/jobs/<id>/beats/x` → 404).
  - The log lines of 2.5.
- **done when:** `cargo build -p calliope-stems && cargo test -p calliope-stems --bins` passes. `--help` shows
  `--beat-tracker` and `--beat-timeout-min`, but not `--beat-timeout-ms`.
- **routine:** no

### Task 4: Protocol conformance and server README for beats
- **files:** `src/calliope-stems/tests/conformance.rs`, `src/calliope-stems/tests/smoke.rs`,
  `src/calliope-stems/README.md`
- **does:**
  - **Conformance:** the real binary on `127.0.0.1:0` with `--beat-tracker ../calliope-gui/tests/support/stub-beat-tracker
    --beat-timeout-ms 500`, raw HTTP and the shared client. Cover:
    - health `tasks` with and without the tracker;
    - `POST /v1/beats` → 202 → done; `…/beats` equals the stub's numbers (27 beats, 7 downbeats for the 16 s
      fixture); 409 while running (`slow`);
    - the 404 cases; 501 without a tracker; 411/413/415;
    - `fail` (message), `bad-output` and `few`;
    - `hang` + `DELETE` (process group gone, folder removed) and `hang` + timeout (failed within about 1 s);
    - a beats job queued behind a `slow` separation;
    - the stem status has no `kind` key.
  - Add cases to the existing tests and helpers rather than new server start-up code.
  - **README:** the endpoints, the beats document, "The beat tracker contract", the flags, and the deployment steps
    of section 5.
- **done when:** `cargo build -p calliope-stems && cargo test -p calliope-stems` passes in under 60 s (only the
  `-ms` knob and the stub modes, no real timeouts).
- **routine:** no

### Task 5: Real tracker adapter, unit example, licence records
- **files:** `src/calliope-stems/trackers/beat-this.sh` (new), `src/calliope-stems/deploy/calliope-stems.service`,
  `src/calliope-stems/tests/smoke.rs`, `docs/licences.md`
- **does:**
  - **The adapter** (2.5): argument and environment checks (exit 2 with clear messages); embedded Python with the
    soundfile read, mono mix, device choice with the CPU fallback on `torch.cuda.OutOfMemoryError`,
    `Audio2Beats`, atomic JSON, progress lines, and `tracker` = `beat_this <version> final0 <device>`.
  - **The unit example:** `Environment=BEATS_HOME=%h/edge-ai/beats` and `--beat-tracker
    %h/.local/bin/calliope-beats-beat-this`.
  - **The test:** `bash -n`, plus the heredoc extracted and parsed with `python3 -I -c "import ast,sys;
    ast.parse(sys.stdin.read())"`.
  - **`docs/licences.md`, server table:**
    - beat-this 1.1.0 (MIT, code and weights);
    - the `final0` checkpoint (MIT; training-data note, accepted by the owner 2026-10-11);
    - torch 2.11.0 (BSD-3-Clause) and torchaudio 2.11.0 (BSD, per its classifier);
    - einops (MIT), rotary-embedding-torch (MIT), soxr (LGPL-2.1-or-later), numpy (BSD-3-Clause), soundfile 0.14
      (BSD-3-Clause; the wheel bundles libsndfile, LGPL-2.1+).

    They are separate processes on the owner's server and never distributed. Rows not read from local metadata are
    marked "per PyPI, 2026-10-11; confirm against the installed `dist-info`".
  - The sentence about the adapter scripts gains the tracker adapter.
- **done when:** `cargo test -p calliope-stems --test smoke` and `cargo test -p calliope-gui --test frontend` (licence
  record test) pass.
- **routine:** no

### Task 6: Tempo map with detected changes (`tempo_map.rs`)
- **files:** `src/calliope-gui/src/tempo_map.rs`
- **does:**
  - **Constants:** `FINE_STEPS_PER_BEAT = 64`, `MAX_OFFSET_STEPS = 256`; `fine_ms`; `tick_ms` made crate-visible.
  - **`Segment.detected_ms: Option<f64>`** (`serde(default, skip_serializing_if = "Option::is_none")`).
  - **`bases()`:** the beat-rounded base for detected changes (2.3). The typed rule is unchanged.
  - **`ms_at` / `tick_near`:** the stretched last beat before a detected change.
  - **`set_change_offset`** (2.3).
  - **`set_bpm_at`:** the anchor rules of the table in 2.3 (keep `detected_ms` on a bpm replace at the start; drop
    it when a segment becomes first; merges drop the later one).
  - **`beats_in`:** unchanged code, verified for detected maps.
  - **Tests:**
    - every existing `tempo_map` test unchanged and passing (typed maps are identical);
    - a detected change at +1, +3, -5 and +256 fine steps: `start_ms` exact; the next section's beats from
      `beats_in` start exactly at `start_ms`; `ms_at`/`tick_near` strictly monotonic and inverse over the stretched
      beat (property loop over all ticks around the change); stretched beat length within [0.5, 1.5] beats;
    - ±257 refused; index 0 refused; a typed segment refused with the exact message; clamping at the 1-beat
      neighbour limits returns the applied steps;
    - after moving change 1, change 2's `start_ms` and `detected_ms` are unchanged;
    - the anchor-rule table, row by row, through `set_bpm_at`.
- **done when:** `cargo test -p calliope-gui --lib tempo_map` passes.
- **routine:** no

### Task 7: `player.json` schema 2
- **files:** `src/calliope-gui/src/player_state.rs`, `src/calliope-gui/tests/acceptance_player_backend.rs`
- **does:**
  - `SCHEMA = 2`; v1 and v2 read, v3+ read-only.
  - `parse_tempo` reads `detected_ms` (finite, `|x| ≤ MAX_SEGMENT_MS`, else dropped) and drops it on the first
    segment.
  - Written files carry `"schema_version": 2`.
  - The existing "newer schema" tests use 3 instead of 2: `player_state.rs` line 493 `{"schema_version": 2, …}` →
    `3`, and `tests/acceptance_player_backend.rs` line 1259 `{"schema_version":2,…}` → `3`.
  - **New tests:** a v1 fixture (`library-player`'s With Tempo) loads with the same map and loops; a round trip keeps
    `detected_ms`; a bad `detected_ms` is dropped while the segment is kept.
- **done when:** `cargo test -p calliope-gui --lib player_state` and `cargo test -p calliope-gui --test
  acceptance_player_backend` pass.
- **routine:** no

### Task 8: Beats to tempo sections (`tempo_detect.rs`)
- **files:** `src/calliope-gui/src/tempo_detect.rs` (new), `src/calliope-gui/src/lib.rs`
- **does:** 2.6. Tests on synthetic beat lists (seeded jitter from a small inline LCG, no new crate):
  - steady 120, with and without ±10 ms jitter → one segment 120.0, anchor within 5 ms of the first downbeat;
  - the stub `ok` pattern → `[(500, 120.0, None), (6500, 90.0, Some(6500))]` exactly;
  - 120 → 90 → 140 → three segments, each change on the left section's fine grid (an integer number of
    `fine_ms(left)` from the left start, within 1e-6);
  - **1-bar sections:** 120 with one bar at 100 in the middle → three sections (120, 100, 120); 120 with a final
    1-bar half-time ending (the stub's `bar-end`) → two sections, the last at 60; with ±10 ms jitter both still hold;
  - a 1-bar section only 3 % off → no section (the 4 % rule);
  - a 4-beat fill made of jitter → nothing; an isolated missed beat and an isolated double beat are repaired with no
    section; a 4-interval half-time run is **not** repaired;
  - 350 BPM folds to 175; no downbeats → anchor at the first beat; unsorted/duplicate input; < 8 beats → exact error;
  - 121.04 BPM for 4 minutes → 121.0, every beat within 60 ms of the grid;
  - 4 500 beats segment in < 200 ms (debug build).

  Record the final `PENALTY` value in the module doc.
- **done when:** `cargo test -p calliope-gui --lib tempo_detect` passes.
- **routine:** no

### Task 9: Mix peaks and debounce (`mix_waveform.rs`)
- **files:** `src/calliope-gui/src/mix_waveform.rs` (new), `src/calliope-gui/src/lib.rs`
- **does:** 2.7. Tests:
  - one stem at 0 dB = `player_source::peaks` of the same PCM;
  - two stems summed, -6 dB, mono→stereo, clipping;
  - zero gains → zeros without iterating;
  - cancel → `None`;
  - `Debounce`: t0+299 no, t0+300 yes and only once; a touch at t0+200 → due at t0+500.
- **done when:** `cargo test -p calliope-gui --lib mix_waveform` passes.
- **routine:** no

### Task 10: Editor: waveform scheduling, peaks event, snapshot
- **files:** `src/calliope-gui/src/editor.rs`, `src/calliope-gui/src/editor_tests.rs`, `src/calliope-gui/src/ipc.rs`,
  `src/calliope-gui/src/editor_ipc_tests.rs`
- **does:** 2.7 in the manager (`set_stem(.., now)`, the worker, generations, cancel on close/switch, `peaks` in the
  snapshot, `set_peaks_fn`). Tests (ManualBackend, injected `Instant`s):
  - changes at t0 and t0+200 → no start at t0+499, exactly one at t0+500;
  - a computation held by a barrier hook while `set_stem`, `play`, `seek` and `snapshot` return and audio is pulled;
  - a superseded result is never emitted;
  - the emitted peaks equal `mix_peaks`;
  - close cancels the worker.
- **done when:** `cargo test -p calliope-gui --lib editor` passes.
- **routine:** no

### Task 11: Tempo store and Player release
- **files:** `src/calliope-gui/src/tempo_store.rs` (new), `src/calliope-gui/src/lib.rs`, `src/calliope-gui/src/player.rs`,
  `src/calliope-gui/src/player_tests.rs`, `src/calliope-gui/src/player_ipc.rs`, `src/calliope-gui/src/editor.rs`,
  `src/calliope-gui/src/editor_tests.rs`
- **does:**
  - **`TempoStore` trait and `FileTempoStore`** (2.2): load; refuse read-only; `apply_tempo_change`; `speed = 1.0`;
    save; never creates the folder.
  - **The Player:**
    - `PlayerManager::release(id) -> Result<bool, String>`: flush, stop, close the output, drop the session, emit
      `PlayerEvent::Closed`, log `player released`;
    - `flush_if_open(id)`;
    - `Loaded.revision` and the re-read in `finish_open`.
  - **`PlayerTempoStore`** in `player_ipc.rs`: `load` = flush-if-open + file load, `replace` = release + file replace,
    both under one `PlayerHost::with`.
  - **The Editor:** `OpenSpec.tempo`, `Session.tempo`, `tempo` in the snapshot, `set_tempo`.
  - **Tests:**
    - file store: loops keep their ms, unknown fields kept, newer schema refused and the file unchanged, no folder
      created;
    - release: a dirty Player session (a loop just added, position moved) is flushed before the write, then the file
      has both the Player's loop/position **and** the new tempo; the session is gone; `Closed` is emitted; the
      session's debounced writer never writes again (tick past 500 ms, file unchanged);
    - a failed flush (track folder removed) refuses the replace;
    - `finish_open` after a concurrent write installs the new tempo (simulate: load, write the file, finish);
    - load with a dirty session flushes it first.
- **done when:** `cargo test -p calliope-gui --lib tempo_store`, `… --lib player` and `… --lib editor` pass.
- **routine:** no

### Task 12: Detection job (`beat_detect.rs`) and manager integration
- **files:** `src/calliope-gui/src/beat_detect.rs` (new), `src/calliope-gui/src/beat_detect_tests.rs` (new),
  `src/calliope-gui/src/lib.rs`, `src/calliope-gui/src/editor.rs`
- **does:** 2.8: params, shared state, phases, part-file cleanup on every path, `start_detect`/`cancel_detect`,
  outcome in `tick`, cancel on close and `shutdown_detect(cap)`, events.

  Tests against the real `calliope-stems` (`target/debug/calliope-stems`, `127.0.0.1:0`, stub separator + stub
  tracker, `--beat-timeout-ms 1000`, poll 50 ms; reuse `import_job_tests.rs`'s start-up code by moving it into a
  shared `#[cfg(test)]` helper module):
  - `ok` on Six Lanes → `Detected`, and `player.json` (schema 2) has `[(500,120),(6500,90,detected 6500)]`; no
    `.calliope-beats-*` left; phases in order;
  - `steady` → one segment; `bar-end` → the last segment at 60;
  - `few`/`fail`/`bad-output` → exact messages, `player.json` unchanged;
  - without `--beat-tracker` → the "cannot detect beats" message; nothing listening → "Cannot reach…";
  - `hang` + cancel → `DetectCancelled`, server job 404, stub process gone, part file gone;
  - `slow` + close → cancelled;
  - a second start refused; no checked stem refused.
- **done when:** `cargo build -p calliope-stems && cargo test -p calliope-gui --lib beat_detect` passes in under 30 s.
- **routine:** no

### Task 13: IPC commands and TS wrappers
- **files:** `src/calliope-gui/src/ipc.rs`, `src/calliope-gui/src/gui.rs`, `src/calliope-gui/build.rs`,
  `src/calliope-gui/capabilities/main.json`, `src/calliope-gui/src/editor_ipc_tests.rs`, `src/ui/lib/ipc.ts`,
  `src/ui/lib/ipc-editor.test.ts`, `src/ui/lib/ipc-player.test.ts`
- **does:**
  - **Commands:** the three of 2.11, with `do_*` functions taking `Arc<dyn TempoStore>`. The app's
    `PlayerTempoStore` is created in `setup`, and `RunEvent::Exit` cancels a detection (2 s cap).
  - **TS:** the types and wrappers, including `PlayerEvent` `closed` and `detected_ms` in segments; mockIPC
    argument-name tests.
  - **`editor_ipc_tests`:** `move` (absolute steps, applied value, the typed-change refusal), `reload`, and the
    `editor_detect` refusals.
- **done when:** `cargo test -p calliope-gui --lib editor_ipc`, `cargo test -p calliope-gui --test frontend` and
  `npx vitest run src/ui/lib/ipc-editor.test.ts src/ui/lib/ipc-player.test.ts` pass.
- **routine:** no

### Task 14: Remove the Player's Detect button
- **files:** `src/ui/components/player/BpmSection.svelte`, `src/ui/components/player/ControlBar.test.ts`,
  `src/ui/acceptance_player.test.ts`
- **does:** exact edits:
  1. In `BpmSection.svelte` delete the line
     `<Button variant="outline" class="h-[3.25rem] text-lg" disabled title={HINT}>Detect</Button>`.
  2. In `ControlBar.test.ts` line 107 change `['Slower', 'Faster', 'Reset', 'Detect']` to `['Slower', 'Faster',
     'Reset']`, and directly after that line add
     `    expect(screen.queryByRole('button', { name: 'Detect' })).toBeNull();`
  3. In `src/ui/acceptance_player.test.ts`:
     - change line 119 `    expect(btn('BPM', 'Detect').disabled).toBe(true);` to
       `    expect(screen.queryByRole('button', { name: 'Detect' })).toBeNull();`;
     - on line 122 change `-, +, Reset and Detect stay disabled` to `-, +, Reset stay disabled`;
     - on line 129 change `['Slower', 'Faster', 'Reset', 'Detect']` to `['Slower', 'Faster', 'Reset']`.
- **done when:** `cd src/calliope-gui && npx vitest run src/ui/components/player/ControlBar.test.ts
  src/ui/acceptance_player.test.ts && npm run check` passes.
- **routine:** yes

### Task 15: Remove the Editor's "BPM & sections" tab
- **files:** `src/ui/lib/views.ts`, `src/ui/lib/views.test.ts`, `src/ui/acceptance_frontend.test.ts`
- **does:** exact edits:
  1. In `views.ts` delete the line
     `  { id: 'tempo', label: 'BPM & sections', feature: 'gui-manipulate-backing-track' },`.
  2. In `views.test.ts` line 62 change `expect(TRACK_TABS).toHaveLength(4);` to `expect(TRACK_TABS).toHaveLength(3);`.
  3. In `acceptance_frontend.test.ts`:
     - line 97: change `['Stems', 'BPM & sections', 'Tablature', 'MIDI cues']` to `['Stems', 'Tablature', 'MIDI
       cues']`;
     - line 92: change `'Editor view has tabs for stems, assembly, BPM/sections, tablature, MIDI cues'` to
       `'Editor view has tabs for stems, tablature, MIDI cues'`.
- **done when:** `cd src/calliope-gui && npx vitest run src/ui/lib/views.test.ts src/ui/acceptance_frontend.test.ts
  && npm run check` passes.
- **routine:** yes

### Task 16: Waveform geometry for sections and drag
- **files:** `src/ui/lib/waveform-geometry.ts`, `src/ui/lib/waveform-geometry.test.ts`
- **does:** the functions of 2.10. `dragSteps`/`offsetSteps` mirror `set_change_offset` (±256, 1-beat neighbour
  limits). The tests reuse Task 6's numbers so frontend and Rust agree.
- **done when:** `npx vitest run src/ui/lib/waveform-geometry.test.ts` passes.
- **routine:** no

### Task 17: Shared WaveformView with sections, drag and keys; Player sections and reopen
- **files:** `src/ui/components/player/WaveformView.svelte`, `src/ui/components/player/WaveformView.test.ts`,
  `src/ui/components/player/PlayerPane.svelte`, `src/ui/lib/player-state.svelte.ts`, `src/ui/lib/player-state.test.ts`,
  `src/ui/views/PlayerView.svelte`
- **does:**
  - **The component:** props; section bars and labels; typed changes dimmed in the Editor; drag with preview and
    readout; the keys of 2.9 (all only with `onmove`).
  - **The Player** passes its segments read-only (no `onmove`), and handles `closed` → reopen.
  - **Tests:**
    - the Player render with loops still draws today's loop marks (existing tests kept), now plus the section bars;
      no interaction without `onmove`;
    - bar x-positions and the `aria-label` (canvas mock records `fillRect`/`fillText`);
    - a drag of N px → one `onmove(1, expectedSteps)`; Escape cancels; typed changes don't start a drag;
    - `→` = +1 (absolute offset+1), `Shift+→` = +64, stops at 256; `↑`/`↓` move the selection between detected
      changes only;
    - player-state: a `closed` event for the open track bumps `pl.reopen` and a later `syncSelection` of the same
      track reopens it (`open_player` called again).
- **done when:** `npx vitest run src/ui/components/player/ src/ui/lib/player-state.test.ts` and `npm run check` pass.
- **routine:** no

### Task 18: Editor frontend state for waveform, tempo and detection
- **files:** `src/ui/lib/editor-state.svelte.ts`, `src/ui/lib/editor-state.test.ts`
- **does:**
  - the state, actions, `canDetect()` (uses the settings state's `edge_ai_url`) and event cases of 2.10;
  - the newest message wins the slot;
  - the log lines.

  Tests (mockIPC):
  - the `canDetect` truth table;
  - `detect` → `detected` (tempo and message, including "replaced");
  - `detect-failed`, `detect-cancelled`;
  - `peaks`;
  - `moveChange` coalescing and absolute steps;
  - stale ids ignored.
- **done when:** `npx vitest run src/ui/lib/editor-state.test.ts` passes.
- **routine:** no

### Task 19: Mix lane UI
- **files:** `src/ui/components/editor/MixLane.svelte`, `src/ui/components/editor/EditorPane.test.ts`,
  `src/ui/views/EditorView.svelte`
- **does:**
  - the layout of 2.9: `WaveformView` with `onmove={moveChange}` and `data-testid="mix-waveform"`;
  - Autodetect BPM/Cancel with its hints; the phase text; the messages;
  - `reloadTempo()` on mount.

  Tests: button states and hints, click → `editor_detect`, Cancel, phase text, messages, the waveform in the Mix lane.
- **done when:** `npx vitest run src/ui/components/editor/` and `npm run check` pass.
- **routine:** no

### Task 20: GUI end-to-end, visual review, docs
- **files:** `src/calliope-gui/tests/gui_editor_e2e.rs`, `src/calliope-gui/tests/common/mod.rs`, `docs/ui.md`,
  `src/calliope-gui/README.md`
- **does:**
  - **Helper:** `start_import_app`'s server script gains `--beat-tracker <stub> --beat-timeout-ms 3000` (harmless for
    import tests). New `start_detect_app(dirs, tracker_mode)`: a `library-editor` copy as the repository, server and
    stub under `unshare -rn`, sharing the start-up code.
  - **Two tests added to `gui_editor_e2e.rs`:**
    1. `mix_waveform_and_autodetect` (Six Lanes, `ok`):
       - a checked stem → an `editor waveform` line with `max>0` within 1 s;
       - three quick toggles → exactly one new `editor peaks` line, ≥ 300 ms after the last change, while a `play`
         is logged at once;
       - all unchecked → Autodetect disabled and `max=0.00`;
       - Autodetect → phases, `editor detected segments=2 bpm=120,90`; `player.json` schema 2 with the detected
         change;
       - screenshots of both themes;
       - focus the waveform, `Shift+→` → `applied=64`, `→` → `applied=65`; `player.json` change start =
         6500 + 65 × 7.8125 ms, `detected_ms` 6500;
       - press `Shift+→` four more times → `applied=256` (limit), file start = 6500 + 256 × 7.8125;
       - drag the bar 40 px back → `editor tempo move`, `|applied| ≤ 256`;
       - Alt+5 (Player): the BPM field shows 120, the V marker is checked, the waveform shows the change bar (screenshot), no
         Detect button.
    2. `autodetect_errors_cancel_and_player_reload`:
       - open the track in the Player and add a loop (`+`), go back to the Editor and run Autodetect (`steady`) →
         `player released`; Alt+5 → `player reopen`, and the loop is still there with the new tempo;
       - `fail` → message;
       - `hang` → Cancel → `editor detect cancelled`, no stub process left;
       - no edge-AI URL → button disabled with its hint;
       - Editor tabs: Stems, Tablature, MIDI cues.
  - **Visual review** at 1280x800 and 1024x640, both themes:
    - waveform height, label legibility at 1-2 m, bar contrast, nothing moving during detection, lanes still
      reachable;
    - in the Player, labels don't collide with the loop marks.
  - **Docs:** the `docs/ui.md` decisions (Mix lane, labels, drag/keys, Player sections, tabs); README usage.
- **done when:** the GUI command with `--test gui_editor_e2e` passes. The screenshots in `target/gui-shots/` were
  reviewed. `npm test` passes.
- **routine:** no

**Order:**
- Server: 1 → 2 → 3 → 4 → 5.
- Rust core: 6 → 7 → 8 → 9 → 10 → 11 → 12 → 13. Tasks 6-9 don't need the server tasks; 12 needs 1-3.
- Frontend: 14 and 15 any time; 16 → 17 → 18 → 19 → 20.

## 4. Test strategy

- **Pure Rust units:**
  - `tempo_map` detected changes, stretch, offsets and anchor rules (Task 6);
  - schema 2 (Task 7);
  - `tempo_detect` on synthetic beats, including 1-bar sections (Task 8);
  - `mix_waveform` (Task 9);
  - `check_beats` (Task 1);
  - the server cli (Task 3).
- **Timing:** injected `Instant`s and a barrier hook for the 0.3 s debounce and the "doesn't block" guarantee (Task
  10); the GUI test only confirms them through log timing.
- **Edge-AI server:**
  - The real `calliope-stems` with the stub separator and the new stub tracker, on `127.0.0.1:0` (Rust tests) or
    inside `unshare -rn` on `127.0.0.1:8765` (GUI tests).
  - Timeouts use `--beat-timeout-ms`. Polling runs at 50 ms in Rust tests.
  - "Old server" = no `--beat-tracker`.
  - Nothing runs Beat This!, PyTorch or the real adapter (syntax-checked only). Nothing touches archserver or the
    laptop's own `calliope-stems` on `0.0.0.0:8765`.
- **Persistence and the Player hand-over:** temp copies, `player.json` on disk before and after; a Player release
  with a dirty session (Task 11); end to end in Task 20.
- **Audio:** nothing new plays (ManualBackend/NullBackend rules unchanged).
- **Frontend:** vitest (geometry, shared waveform, editor and player state, Mix lane, removed button and tab) and
  `svelte-check`.
- **GUI:** two new tests in `tests/gui_editor_e2e.rs`, plus the screenshots.
- **Whole suite:** `cd src/calliope-gui && npm test`, then `npm run test:gui`.

## 5. Deployment

| Component | Runs on | How it gets there |
|---|---|---|
| calliope-gui | laptop | `cd src/calliope-gui && npm run build:app` |
| calliope-stems (beats endpoints) | archserver | rebuilt and restarted by the owner |
| `trackers/beat-this.sh` + Beat This! venv + checkpoint | archserver | installed by the owner |

The new app reads and writes `player.json` schema 2, so older app builds open such files read-only.

**archserver steps (owner only):**
1. `git pull && cargo build --release -p calliope-stems`
2. Create the environment:
   ```bash
   uv venv --python 3.12 ~/edge-ai/beats/.venv
   uv pip install --python ~/edge-ai/beats/.venv/bin/python \
       --index-url https://download.pytorch.org/whl/cu128 torch==2.11.0 torchaudio==2.11.0
   uv pip install --python ~/edge-ai/beats/.venv/bin/python beat-this==1.1.0 soundfile
   mkdir -p ~/edge-ai/beats/models
   curl -fL -o ~/edge-ai/beats/models/final0.ckpt \
       https://cloud.cp.jku.at/public.php/dav/files/7ik4RrBKTS273gp/final0.ckpt
   ```
3. Install the binaries:
   ```bash
   install -m755 target/release/calliope-stems ~/.local/bin/
   install -m755 src/calliope-stems/trackers/beat-this.sh ~/.local/bin/calliope-beats-beat-this
   ```
4. Smoke test: `BEATS_HOME=~/edge-ai/beats calliope-beats-beat-this song.flac /tmp/bt && head -c 300
   /tmp/bt/beats.json` prints progress lines, then the JSON with `beats`, `downbeats` and `tracker` (which names the
   device).
5. In `~/.config/systemd/user/calliope-stems.service`, add `Environment=BEATS_HOME=%h/edge-ai/beats` and
   `--beat-tracker %h/.local/bin/calliope-beats-beat-this` (as in the example unit). Then:
   `systemctl --user daemon-reload && systemctl --user restart calliope-stems`.
6. From the laptop: `curl http://archserver:8765/v1/health` shows `"tasks":["stems","beats"]`.
7. Optional: compare `~/edge-ai/beats/.venv/lib/python3.12/site-packages/*.dist-info/METADATA` licences with
   `docs/licences.md`.

## 6. Manual checks (owner: laptop + archserver)

- **M1:** Real stem track in the Editor:
  1. Check drums + bass + vocals: the waveform appears within about 1 s.
  2. Uncheck the drums: it changes about 0.3 s after the last click, and playback never stutters.
- **M2:** Steady song:
  1. Autodetect BPM: the status goes Preparing → Uploading → Detecting → Saving, then "Detected 1 tempo: N BPM".
  2. In the Player, Metronome On + Play: the clicks stay on the beat to the end.
- **M3:** Song with a tempo change:
  1. An amber bar near the change, with the new BPM, in both the Editor and the Player.
  2. In the Player, V is checked and the metronome follows the change.
- **M4:** A song that ends with a 1-bar change (e.g. a half-time last bar): a separate section is shown for it.
- **M5:** Fine adjustment:
  1. Drag a change bar: it moves, and stops 4 beats from where it was detected, also across several drags.
  2. Click the waveform: ←/→ moves one 1/64 beat, Shift+←/→ one beat.
  3. In the Player, the metronome's first click of the new section lands on the moved bar.
- **M6:** Player hand-over:
  1. In the Player, add a loop and pause mid-song; switch to the Editor and run Autodetect.
  2. Back in the Player: the track reloads at the same position, the loop sits on the same music, and the new tempo
     shows.
- **M7:** Start Autodetect and Cancel at once: "cancelled", the server journal shows the job cancelled, and no
  `python` process is left. With the server stopped, the message says it can't be reached.
- **M8:** Note the detection time for a 4-minute song. Optionally check `nvidia-smi` (GPU, or the CPU fallback).
- **M9:** The Player has no Detect button, and the Editor has no "BPM & sections" tab.

## 7. Acceptance mapping (accepted by the owner; A10, A11, A13, A15 adjusted; A16, A17 added)

| # | Criterion | Spec | Tasks | Tests |
|---|---|---|---|---|
| A1 | Given an active stem track in the Editor, then the Mix lane shows a full-width waveform of the current mix (checked stems at their volumes) with a playhead | req 1 | 10, 17, 19 | editor_tests, EditorPane.test, gui |
| A2 | Given a stem's checkbox or volume changes, when 0.3 s pass without another change, then the waveform is recomputed once and redrawn; meanwhile Play/Pause, seeking and the other lanes respond at once and the audio does not stop | S1 | 9, 10, 20 | mix_waveform, editor_tests (barrier), gui |
| A3 | Given a change, when another change comes within 0.3 s, then the wait restarts from the last change and only one recomputation happens | S2 | 9, 10 | editor_tests, gui |
| A4 | Given no stem is checked (or all checked stems are Off), then the waveform is flat and Autodetect BPM is disabled with a hint; given at least one checked stem above Off and an edge-AI server set, then it is enabled | S3, req 2 | 18, 19 | editor-state, EditorPane.test, gui |
| A5 | Given Autodetect BPM is clicked, then the current mix is rendered and sent to calliope-stems (`POST /v1/beats`); phases and Cancel are shown; playback keeps working | S4 | 1, 3, 12, 19 | conformance, beat_detect_tests, gui |
| A6 | Given a detection is running, when Cancel is pressed (or the track is switched, or the app closes), then the server job is deleted, no file changes, and the button is available again | | 12, 13, 19 | beat_detect_tests, gui |
| A7 | Given the server is unreachable, has no beat detection, fails, or finds fewer than 8 beats, then an inline message says so and the tempo map is unchanged | | 12, 18 | beat_detect_tests, editor-state, gui |
| A8 | Given a steady song, when detection succeeds, then the track has one tempo starting at the first downbeat, and the waveform shows "N BPM" at the left edge with no bar | req 3 | 8, 12, 17 | tempo_detect, beat_detect_tests, WaveformView.test |
| A9 | Given a song with several tempos, including sections as short as 1 bar that differ by more than 4 %, when detection succeeds, then each change is placed on the nearest 1/64-beat step of the previous section's tempo, and the waveform shows an amber bar at each change followed by its BPM | S5, req 3-4 | 8, 17 | tempo_detect (incl. 1-bar), WaveformView.test, gui |
| A10 | Given detection succeeded, then within 1 s `player.json` (schema 2) holds the new map with each change's detected position; the previous map is replaced; loops keep their time, the Player's position and other settings are kept, speed is 100 %; after a restart the Player and the Editor show it | S6 | 6, 7, 11, 12 | tempo_store, player_state, beat_detect_tests, gui |
| A11 | Given the Player has the track open, when the Editor writes a new or moved tempo map, then the Player's pending settings are saved first, its session is closed, and the next time the Player shows the track it reloads from `player.json` with the new tempo and its loops and position intact; no later Player write brings back the old tempo | owner 8a | 11, 17 | player_tests (release), tempo_store, player-state.test, gui |
| A12 | Given a detected change, when the user drags its bar, then it moves in 1/64-beat steps of the left section's tempo, at most 4 beats from its detected position in total over any number of drags, never within one beat of its neighbours; the result is saved on release and later changes keep their time | S7, req 4 | 6, 16, 17, 11 | tempo_map, waveform-geometry, WaveformView.test, gui |
| A13 | Given the waveform has the focus, when ↑/↓ select a detected change and ←/→ (1/64 beat) or Shift+←/→ (1 beat) are pressed, then the same total ±4-beat limit from the detected position applies | req 4 | 17, 20 | WaveformView.test, gui (keys to 256) |
| A14 | Given the Player view, then the BPM section has no Detect button | out of scope | 14 | ControlBar.test, acceptance_player.test |
| A15 | Given a BPM was typed in the Player, when the user returns to the Editor, then the waveform shows the current sections; detected positions follow the rules of plan 2.3 (kept on a BPM correction at a change, dropped with a removed or merged change, replaced by a new detection) | | 6, 11, 19 | tempo_map (anchor table), editor_ipc_tests (`reload`), EditorPane.test |
| A16 | Given a track with a tempo map, when it is shown in the Player, then the Player's waveform shows the start BPM at the left edge and an amber bar with the new BPM at each change, read-only (no drag, no keys), next to the loop marks | owner 8c | 17, 20 | WaveformView.test, gui screenshot |
| A17 | Given the Editor view, then its tabs are Stems, Tablature and MIDI cues (no "BPM & sections") | owner 10 | 15 | views.test, acceptance_frontend.test, gui |

## 8. Owner decisions (2026-10-11; no open questions)

1. **Storage:** the tempo map is in `player.json` (2.2).
2. **What is analysed:** the current Editor mix (2.4).
3. **Method:** Beat This! on archserver; the training-data note is accepted (2.5).
4. **Step:** 1/64 of a beat (4 beats = 256 steps). Changes are stored in ms with a detected position; detected
   changes start their section on a beat, with a stretched last beat; the loop grid is unchanged; `player.json`
   becomes schema 2 (2.3).
5. **Limit:** ±4 beats from the detected position, in total. The detected positions are stored, with the rules for
   typed BPMs and re-detection in 2.3.
6. **Shortest section:** 1 bar; optimal segmentation with the 4 % rule (2.6).
7. **Autodetect enabled:** at least one checked stem above Off, plus a server set in Settings.
8. **Player:**
   - (a) its session is flushed and closed when the Editor writes the tempo, and it reloads later (2.2);
   - (b) a shared props-driven WaveformView;
   - (c) sections shown read-only in the Player's waveform.
9. **Replacing a map:** no dialog, the result line says so, no undo.
10. **Tabs and labels:** the "BPM & sections" tab is removed; "120 BPM" labels and amber bars.
11. **Criteria:** A1-A15 accepted; A10, A11, A13 and A15 adjusted; A16 and A17 added.
