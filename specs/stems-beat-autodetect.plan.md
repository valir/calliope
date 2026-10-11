# Plan: Stems beat autodetection (Editor mix waveform, Autodetect BPM, tempo sections)

Revision 1 (architect). Base `f03dcfd`, branch `spec/stems-beat-autodetect`. Builds on
`specs/gui-backing-track-editor.plan.md` (Editor engine, Mix lane), `specs/gui-play-backing-track.plan.md`
(tempo map 2.3, `player.json` 2.5, waveform 2.10, the detection sketch in 2.13) and
`specs/gui-stem-extraction.plan.md` (calliope-stems protocol, separator process model, stub separator).

Decisions marked **(rec)** are the architect's recommendation and wait for the owner (section 8). The tasks are
written for the recommendations; section 8 says which tasks change if an answer differs.

## 1. Summary

The Editor's Mix lane gets the Player's waveform. Rust draws it from the current mix: the checked stems at their
volumes. It is recomputed on a worker thread 0.3 s after the last change to the mix, and every new change restarts
that wait. A new **Autodetect BPM** button renders the current mix and sends it to `calliope-stems` on archserver.
There "Beat This!" (CPJKU, MIT code and weights) finds the beats and downbeats, behind a new additive
`/v1/beats` endpoint and a "beat tracker" adapter command. The app turns the beats into tempo sections, and each
tempo change sits on the 1/64 grid of the section before it. The sections are saved as the track's tempo map in
`player.json`, the one the Player already uses for its metronome and count-in. The waveform shows the start BPM at
the left edge, and an amber bar followed by the new BPM at each change. A bar can be dragged, or moved with the
keys, in 1/64 steps of the left section's tempo, at most 4 beats per move. In the Player, the disabled "Detect"
button is removed.

## 2. Design

### 2.1 What is reused, what is new

Reused unchanged:
- `tempo_map` (segments, 1/64-note ticks, `tick_near`/`ms_at`, `set_bpm_at` snapping rule);
- `player_state` (load/save `player.json`, `apply_tempo_change` re-snapping loops);
- `backing_render::render_flac` (renders the mix to a FLAC);
- `mixer::{gain_factor, to_stereo_f32, mix}`;
- the calliope-stems job queue, the process runner and the stub-separator pattern;
- `stems_client` (health, `wait_final`, `delete`);
- the editor channel and its event pattern;
- `bpm-format.ts` and `waveform-geometry.ts`.

New or changed:

| Where | What | Pure? |
|---|---|---|
| `src/calliope-lib/src/stems_api.rs` | `Health.tasks`, `JobStatus.kind` (both additive), `Beats {beats, downbeats, tracker}`, `check_beats(&Beats, duration_s)`, limits | yes |
| `src/calliope-lib/src/stems_client.rs` | `supports_beats(&Health)`, `submit_beats(file, cancel, on_sent)`, `fetch_beats(job, duration_s)` | client |
| `src/calliope-stems/src/{cli,config,jobs,server}.rs` | `--beat-tracker PATH`, `--beat-timeout-min`, hidden `--beat-timeout-ms`; job kind `beats`; `POST /v1/beats`, `GET /v1/jobs/<id>/beats`; `tasks` in health | server |
| `src/calliope-stems/trackers/beat-this.sh` | the real tracker adapter: bash + embedded Python; runs Beat This! from the owner's venv | archserver |
| `src/calliope-gui/tests/support/stub-beat-tracker` | test stand-in (Python 3 stdlib): canned beats computed from the input's duration, failure modes | test |
| `src/calliope-gui/src/tempo_map.rs` | `move_change(index, ticks)`: moves a section start along the left section's grid, re-snaps later changes | yes |
| `src/calliope-gui/src/tempo_detect.rs` | beats + downbeats → `TempoMap` (sections, quantised BPM, 1/64 snapping) | yes |
| `src/calliope-gui/src/mix_waveform.rs` | `mix_peaks(stems, gains, frames, buckets, cancel)`, `Debounce` (0.3 s restartable deadline) | yes |
| `src/calliope-gui/src/tempo_store.rs` | `trait TempoStore { load, replace }`, `FileTempoStore` (`player.json`) | yes (file IO) |
| `src/calliope-gui/src/beat_detect.rs` | the detection job: render mix → upload → wait → fetch → `tempo_detect` → outcome | job thread |
| `src/calliope-gui/src/editor.rs` | peaks scheduling and worker, `tempo` in the session, detection start/cancel/outcome, new events | manager |
| `src/calliope-gui/src/player.rs`, `player_ipc.rs` | `PlayerManager::replace_tempo` + `PlayerEvent::Snapshot`; `PlayerTempoStore` (the app's `TempoStore`) | (Player internals, Q8) |
| `src/calliope-gui/src/ipc.rs`, `gui.rs`, `build.rs`, `capabilities/main.json` | `editor_detect`, `editor_cancel_detect`, `editor_tempo`; exit cancels detection | Tauri glue |
| `src/ui/components/player/WaveformView.svelte` | props instead of reading `pl`; optional sections, drag and keys; the Player passes the same data as today | component |
| `src/ui/lib/waveform-geometry.ts` | `sectionMarks`, `labelFits`, `dragTicks`, `tickMs` | yes |
| `src/ui/lib/editor-state.svelte.ts`, `components/editor/{MixLane,EditorPane}.svelte`, `views/EditorView.svelte` | waveform in the Mix lane, Autodetect BPM button, progress/Cancel, messages, tempo reload on entry | frontend |
| `src/ui/components/player/BpmSection.svelte` | the "Detect" button is removed | frontend |

### 2.2 Where the tempo map lives (Q1, rec)

**One tempo map per track: `tempo` in `player.json`, shared by the Editor and the Player.**
- **Why:**
  - The spec's goal is the metronome and the count-in, and those read `player.json`'s tempo.
  - The stems, the backings and the original are sample-aligned, so one map fits every source (the owner's
    "one state per track" decision).
  - A second copy, for example in `track.json` or per backing variant, would have to be kept in sync with the
    Player's map, which the user can still edit by typing a BPM.
- **"Save the information along the mix track"** becomes: the map is written to `player.json` in the track folder
  (next to `track.json`, `stems/` and `backings/`), within 1 s.
- **Not stored:** the raw beat list and the detection's provenance. They are not needed to play, and they would
  bloat a hand-editable file. A re-detection is one click.
- **Replacing a map** is the existing Player rule (`apply_tempo_change`):
  - loops keep their time and are re-snapped to the new grid;
  - `speed` resets to 1.0, as when a BPM is typed (speed is always 1.0 today);
  - `beats_per_bar` keeps its stored value (4);
  - every other field is kept.
- **A detection replaces the whole map** (Q9, rec: no confirmation dialog). The result line says "Detected 2 tempos:
  120, 90 BPM (replaced the previous tempo map)". There is no undo.
- **A newer `player.json`** (schema > 1) is never overwritten: detection and drag fail with "player.json was written
  by a newer version; the tempo was not saved".
- **Concurrency with the Player:**
  - `TempoStore::replace(id, dir, f)` runs under the `PlayerHost` lock.
  - If the Player has this track open, the change goes through its session: `PlayerManager::replace_tempo`
    re-snaps the loops, rebuilds the beat grid and marks the session dirty. Its debounced writer saves the file,
    and a `PlayerEvent::Snapshot` refreshes the Player's UI.
  - Otherwise, `FileTempoStore` does load → apply → `write_atomic` of `player.json`, which never creates the
    folder.

  So the two views can never overwrite each other's tempo.
  - The Editor reads the map through the same store when it opens a track, and again each time the Editor view is
    shown (`editor_tempo` `reload`). A BPM typed in the Player therefore appears in the Editor.
  - **Accepted limit:** the Player reads `player.json` off-lock while opening. A write that lands between that read
    and `finish_open` could be lost. It can't happen in practice: the Player only opens a track while its own view
    is shown, and detection runs from the Editor.

### 2.3 What is analysed (Q2, rec)

**The current Editor mix.** This means the checked stems at their volumes, with solo ignored, the same mix that Save
would write (`render_flac` re-reads the stems at full precision).
- The spec says "the autodetect algorithm is being run on the mix".
- The user can include the drums for a clearer beat.
- The mix need not have been saved as a backing first.

The mix is rendered to `<track>/.calliope-beats-<uuid>.part`, a dot name the scans ignore (same pattern as
`.calliope-backing-*.part`), and is removed after the upload, on failure, on cancel and on exit. Rendering a
4-minute mix of 6 stems takes a few seconds in a release build. The FLAC is stereo at the stems' rate (≤ about
100 MB for 15 minutes, under the server's 300 MB cap). A mix longer than `MAX_DURATION_S` (900 s) is refused
before rendering: "Tracks longer than 15 minutes can't be analysed".

### 2.4 Detection method: Beat This! on archserver (Q3, rec)

**Candidates:**

| Option | Licence | Accuracy, tempo changes | Cost |
|---|---|---|---|
| **Beat This! 1.1.0 (CPJKU), `final0` checkpoint (rec)** | code **and** published weights MIT (README: "The code and the published model weights are released under the MIT license"; PyPI `License-Expression: MIT`) | state of the art for beats and downbeats; no tempo-constancy assumption, so it follows tempo changes | PyTorch on archserver (already there for Demucs); GPU with CPU fallback |
| A local non-AI detector in Rust (onset envelope + autocorrelation + dynamic programming) | ours | weaker on real music; tempo changes need extra heuristics | no server; a tricky algorithm to tune without real data |
| madmom (DBN) | code BSD, **models CC BY-NC-SA** | good | non-commercial weights; rejected |
| aubio / Essentia | GPL-3 / AGPL-3 | fair | copyleft; rejected |
| librosa `beat_track` | ISC | assumes one tempo | rejected for multi-tempo songs |

Beat This! was trained on datasets some of which are copyrighted or under limited CC licences. Upstream says users
should check whether this affects them. The weights themselves are MIT and the use is personal, on the owner's own
server. This is recorded in `docs/licences.md`.

**Runtime on archserver** (the owner installs; agents never do):
- A **separate uv venv** `~/edge-ai/beats/.venv` (Python 3.12) with `torch==2.11.0` and `torchaudio==2.11.0` from
  the cu128 index, then `beat-this==1.1.0` and `soundfile`.
  - A separate venv means an upgrade can never break the working Demucs environment.
  - It costs about 4 GB of disk (80 GB free).
- **The checkpoint is downloaded once** to `~/edge-ai/beats/models/final0.ckpt` (about 78 MB), so the service never
  downloads at run time.
- **The adapter uses CUDA when available, otherwise the CPU.**
  - On a CUDA out-of-memory error it retries on the CPU, because Ollama may hold most of the 16 GB.
  - Beat This! is small (about 20 M parameters). A 4-minute song takes seconds on the GPU, and tens of seconds on
    the CPU (to be confirmed by the owner, manual check M6).
- **Not used:** madmom (only for the optional `--dbn`) and Ollama.

**Protocol additions** ("calliope-stems API v1", additive; older apps and servers are unaffected):
- **`GET /v1/health`** gains `"tasks": ["stems"]`, or `["stems","beats"]` when `--beat-tracker` is set.
  - An app that finds no `"beats"` (an old server, or no tracker) says: "The edge-AI server cannot detect beats yet;
    update calliope-stems on the server (see its README)".
- **`POST /v1/beats`**: the body is a FLAC; `Content-Type: audio/flac` and `Content-Length` are required.
  - It returns `202 {"job","state":"queued"}`.
  - It applies the same checks as `POST /v1/jobs` (411, 413 for size or duration, 415, 503 when the queue is full),
    and `501 {"error":"beat detection is not configured on this server"}` without a tracker.
  - Beat jobs share the one queue and the one worker with separation jobs (one GPU).
- **`GET /v1/jobs/<id>`** works for both kinds. A beats job's status has `"kind":"beats"` and `"stems": null`. For
  stem jobs the key is absent, so their JSON stays byte-identical to today's.
- **`GET /v1/jobs/<id>/beats`**: `200 {"beats":[s...],"downbeats":[s...],"tracker":"..."}` once the job is `done`.
  - Otherwise `409` if it is not done yet, and `404` if the job is unknown or not a beats job.
  - `GET /v1/jobs/<beats-job>/stems/<n>` → 404.
- **`DELETE /v1/jobs/<id>`** cancels either kind: it kills the tracker's process group and removes the folder.

**Beats document** (`stems_api::Beats`), validated by `check_beats(&b, duration_s)` on the server (tracker output)
and in the client (after fetch):
- times in seconds from the start of the file, finite, `0 ≤ t ≤ duration_s + 1`, strictly increasing;
- at most 20 000 beats;
- every downbeat equals a beat within 1e-6 s;
- `tracker` is optional text of ≤ 100 characters (for logs);
- the JSON is ≤ 1 MiB. The client's general 64 KiB JSON cap does not apply to this endpoint: 4 500 beats are about
  50 KB.

**Tracker contract** (like the separator's):
- It is called as `<tracker> <input.flac> <out_dir>`, in the job folder.
- It writes `<out_dir>/beats.json` (the beats document) and exits 0.
- stdout may carry `progress <0..1>` lines, which the server relays. Anything else is logged.
- **Failures:** any other exit status, a missing or invalid file, or a timeout fail the job, with the message (and
  the stderr tail) going to the client.
- **Limits:** `--beat-timeout-min` defaults to 10. The hidden `--beat-timeout-ms` (minimum 100) is for tests.
- The server kills the tracker's process group on cancel, on timeout and when it exits.
- No `stem_levels` measurement runs for beat jobs.
- **Log lines:** `job id=… kind=beats state=…` and `job id=… beats=<n> downbeats=<m> tracker="…"`.

**Real adapter** `src/calliope-stems/trackers/beat-this.sh`:
- **Environment:** `BEATS_HOME` (default `~/edge-ai/beats`), the Python `$BEATS_HOME/.venv/bin/python`, and the
  checkpoint `$BEATS_HOME/models/final0.ckpt` (override with `BEAT_THIS_CHECKPOINT`).
- **What it does:** it checks its arguments and runs an embedded Python program with `"$PY" - "$@" <<'PY'`. The
  program:
  - reads the FLAC with `soundfile` and mixes it down to mono;
  - prints `progress 0.1`;
  - runs `beat_this.inference.Audio2Beats(checkpoint_path, device, dbn=False)`;
  - prints `progress 0.9`;
  - writes `beats.json` atomically (tmp + rename).
- It is our own code, which uses the MIT library. Tests only syntax-check it: `bash -n`, plus `ast.parse` of the
  extracted heredoc with `python3 -I`. No test runs it.

### 2.5 From beats to tempo sections (`tempo_detect.rs`)

`detect(beats_s: &[f64], downbeats_s: &[f64], beats_per_bar: u32) -> Result<Detected { map: TempoMap, sections:
Vec<SectionInfo { first_beat, last_beat, bpm }> }, String>`, pure.

Constants (Q6, rec): `MIN_BEATS = 8`, `WINDOW = 8` intervals, `CHANGE = 0.04` (4 %), `SUSTAIN = 8` intervals,
`MIN_SECTION_BEATS = 8`, `MAX_SECTIONS = 32`.

1. **Clean the input.**
   - Drop non-finite and negative values, sort, and drop duplicates (within 1 ms).
   - Fewer than `MIN_BEATS` beats → `Err("No steady beat was found in this mix; check more stems (the drums help)
     and try again")`.
2. **Local period.** For each inter-beat interval i: `p_i` = the median of the intervals in `[i-4, i+4)`, clamped to
   the ends. The median ignores single fills and missed beats.
3. **Find the changes** (a greedy pass).
   - Start a section at interval 0 with the reference period `P` = the median of its first `WINDOW` intervals.
   - A **change** starts at interval c when `|p_i / P - 1| > CHANGE` holds for `SUSTAIN` consecutive intervals from
     c on. Beat c is then the first beat of the new section, with `P` re-estimated over its first `WINDOW`
     intervals.
   - Brief deviations (fills, a ritardando shorter than 2 bars) don't create sections.
4. **Merge short sections.** A section shorter than `MIN_SECTION_BEATS` beats is merged into its shorter neighbour.
   If there would be more than `MAX_SECTIONS`, keep the largest changes.
5. **BPM per section.**
   - A least-squares fit of beat time against beat index gives the period T.
   - `bpm = round(600 / T) / 10` (one decimal, the tempo map's resolution).
   - The value is folded into 20..300 by doubling or halving (a half- or double-time beat track).
   - Neighbouring sections with the same rounded BPM are merged.
6. **Anchor (bar 1, beat 1).**
   - Take the first downbeat inside section 0, or the first beat if there is none.
   - Refine it with the intercept of a fixed-slope fit (slope = the quantised period) over section 0's beats, moved
     to that beat's index. This centres the quantisation drift: at ±0.05 BPM over a 4-minute section, the grid
     stays within about ±60 ms of the beats.
   - The first segment is `{start_ms: anchor, bpm_0}`, and its tempo extends backwards (existing rule).
7. **The other sections**, in order. For section k ≥ 1:
   - the fitted start is the intercept of its fixed-slope fit at its first beat;
   - `tick = map.tick_near(start)`; `start_ms = map.ms_at(tick)` on the map built so far, so **the change sits on
     the nearest 1/64 tick of the previous section's grid** (spec req. 4, the same rule as `set_bpm_at`);
   - push `{start_ms, bpm_k}`.

   A section that ends up less than one beat after the previous one is merged.
8. **Bars.** `beats_per_bar` keeps the stored value (4, owner decision). The downbeats only set the anchor. The
   number of beats per bar the tracker found most often is logged (`bar_beats=<n>`) for the later time-signature
   work.

The result is logged as `calliope: editor detected id=… beats=<n> downbeats=<m> bar_beats=<b> segments=<k>
bpm=<a,b,..> starts_ms=<..>`.

### 2.6 Moving a tempo change (`tempo_map::move_change`)

Spec: "allow that bar movement left or right in increments of 1/64 of the left section's BPM but for no more than 4
beats".

`fn move_change(&self, index: usize, ticks: i64) -> Result<(TempoMap, i64 /* applied */), String>`
- `index ≥ 1`. The first segment is the anchor, has no bar and can't be moved.
- **The step** is one tick of segment `index-1`: a 1/64 note, i.e. 1/16 beat (Q4). The new start is
  `ms_at(base_i + ticks)` evaluated on segment `index-1`'s grid. Every step therefore stays on the left section's
  1/64 grid.
- **`|ticks| ≤ MAX_MOVE_TICKS = 64`** (4 beats) per call. Larger values are refused (`Err`), because a frontend
  error should not be hidden.
- **Clamped, not refused:** the new start stays at least one beat (16 ticks of segment `index-1`) after segment
  `index-1`'s start, and at least one beat (of segment `index`) before segment `index+1`'s start. The applied tick
  count is returned.
- **Later changes stay with the audio.** Each later start is re-snapped, in order, to the nearest tick of its
  previous segment's new grid (it moves by at most half a tick, ≤ 16 ms at 60 BPM). Loops are re-snapped by
  `apply_tempo_change` as usual.

**The 4-beat limit counts per move** (Q5, rec): per drag gesture, or per keyboard selection.
- A drag starts from the bar's position at pointer-down and is clamped to ±64 ticks. The frontend preview and Rust
  apply the same clamp.
- With the keyboard, the limit counts from the bar's position when it was selected. Each key press sends one step,
  and the frontend stops sending once the selection is 64 ticks away.
- So a badly detected change can still be moved further in several moves. The alternative (at most 4 beats from the
  *detected* position, for good) would need the detected positions stored; see Q5.

### 2.7 The mix waveform and its debounce (spec AC 1-2)

- **What it shows:** the mix of the stems that are checked, at their volumes (`mixer::gain_factor`), solo ignored,
  hard-clipped like playback. So the waveform is what Mix Play and Save produce.
- **How it is computed:** `mix_waveform::mix_peaks(stems: &[StemPcm], gains: &[f32], total_frames, buckets:
  2048, cancel: &AtomicBool) -> Option<Vec<f32>>` mixes 4096-frame chunks with `to_stereo_f32` + `mix` from the
  in-memory i16 stems. It takes the maximum |sample| per bucket, with the same bucket rule as `player_source::peaks`
  (`per = frames / buckets`, the last bucket takes the rest).
  - It checks `cancel` between chunks, and returns `None` when cancelled.
  - All gains 0 gives zeros at once, with no pass over the audio.
- **Debounce in Rust** (the frontend never keeps time): `mix_waveform::Debounce { due: Option<Instant> }`, with
  `DELAY = 300 ms`.
  - `touch(now)` sets `due = now + 300 ms`, so a change inside the window restarts it.
  - `take_due(now) -> bool` fires once when `now ≥ due`.
  - `EditorManager::set_stem(id, name, gain, unmuted, now)` (it gains `now`) calls `touch(now)`. So does any
    change to the gains, including a restored mix at open. At open the first computation is due at once (no 0.3 s
    wait).
- **Running it without blocking anything.** When `tick(now)` finds the debounce due, it:
  - cancels a computation still running (sets its flag);
  - increments `peaks_gen`;
  - spawns a worker thread with the `Arc` of the in-memory stems and a copy of the gains.

  The worker never takes the manager lock. On success it stores `(gen, peaks)` into a shared slot. The next tick
  (≤ 20 ms later) installs it into the session only if `gen` is still current, and then emits
  `EditorEvent::Peaks {id, peaks}`.
  - IPC commands, playback and the audio callback never wait for it.
  - A session close or a track switch cancels the worker.
  - A test hook (`EditorManager::set_peaks_fn`, `#[doc(hidden)] pub`) replaces `mix_peaks`, so tests can hold a
    computation open deterministically.
- **Snapshot:** `peaks: number[]` (the latest, empty before the first result). The UI keeps drawing the old peaks
  until the new ones arrive (no flash).
- **Log:** `calliope: editor peaks id=… gen=<n> ms=<compute time>`.

### 2.8 The detection job (`beat_detect.rs`) and its states

`EditorManager::start_detect(id, params: DetectParams { url, store: Arc<dyn TempoStore>, poll, stall })`:
- **Starting it** is refused with:
  - "Check a stem first" when no checked stem is above Off (Q7);
  - "Beat detection is already running" while one runs (one detection per app; it can run during playback and
    during a backing Save);
  - "Set the edge-AI server in Settings first" when no URL is configured.
- **The job thread** (shared state `DetectShared`, the same shape as `SaveShared`) runs these phases, each reported
  as `EditorEvent::Detect {id, phase, progress}`, throttled to ≤ 10 per second:
  1. `checking`: `health()`; it fails unless `tasks` contains `beats`.
  2. `mixing` (0..1): `render_flac` of the checked stems into the `.part` file.
  3. `uploading` (bytes sent / total): `submit_beats`; the part file is removed right after.
  4. `waiting`: the server job is `queued` (for example behind a separation). The UI says "Waiting for the edge-AI
     server...".
  5. `detecting` (the server's progress, or none): the job is `running`.
  6. `applying`: `fetch_beats` → `tempo_detect::detect` → `store.replace(id, dir, |old| new)`.
- **Polling:** `wait_final(job, cancel, poll, stall)`, where `poll` is `ipc::IMPORT_POLL` (1 s) in the app and
  50 ms in tests, and `stall` is `STALL_TIMEOUT`.
- **The outcome** (`Ok(TempoMap)` or `Err(message)`, or cancelled) goes into `DetectShared`. The manager's `tick`
  installs it and emits exactly one final event:
  - `Detected {id, tempo, summary}` after updating `session.tempo`;
  - `DetectFailed {id, message}`;
  - `DetectCancelled {id}`.
- **Cancel:** `cancel_detect(id)`, a track switch, `close` and app exit (2 s cap, like the save) set the flag.
  - The client then stops uploading or polling and `DELETE`s the server job (best effort).
  - The part file is removed, and nothing is written.
  - A cancel that arrives after `applying` has started is too late: the map is already saved, and the job ends as
    `Detected`.
- **Messages** (inline, in the Mix lane's reserved slot):
  - unreachable: "Cannot reach the edge-AI server at <url>";
  - no `beats` task: as in 2.4;
  - 413: "The server refused the mix: <server message>";
  - tracker failure: "Beat detection failed on the server: <message>";
  - stalled: "The edge-AI server stopped answering; try again";
  - too few beats: see 2.5;
  - newer `player.json`: see 2.2;
  - success: "Detected N tempos: a, b BPM", or "Detected 1 tempo: 120 BPM".

### 2.9 Behaviour and layout (Editor)

- **Mix lane, top to bottom:**
  1. the waveform: full width, about 4 rem high, final size set by the visual review;
  2. Mix cell, Play/Pause, Stop, then **Autodetect BPM** right-aligned. While a detection runs, the button reads
     **Cancel**, and a status text shows the phase and its percentage ("Preparing the mix 40%", "Uploading 70%",
     "Waiting for the edge-AI server", "Detecting beats", "Saving the tempo");
  3. the position slider and time field;
  4. Save and "Save as";
  5. the reserved message slot, which the save result and the detection result/errors share. The newest message
     wins.

  Nothing moves when state changes (the existing rule).
- **Autodetect BPM is enabled** when the pane is active, at least one stem is checked with a volume above Off (Q7),
  the edge-AI URL is set, and no detection is running.
  - Without a URL it is disabled, and its hint reads "Set the edge-AI server in Settings to detect the BPM".
  - Without a stem the hint reads "Check a stem to detect its BPM".
- **The waveform drawing** (same canvas component as the Player, 2.10):
  - **peaks:** the waveform in `--muted-foreground`;
  - **playhead:** a 1 px `--foreground` line;
  - **tempo:** the start BPM label at the left edge (no bar), then for each change a 2 px amber (`--primary`)
    full-height bar with the new BPM label right of it;
  - **labels:** "120 BPM" if it fits before the next bar, else "120", else omitted (the `aria-label` always lists
    every tempo, e.g. "Mix waveform, tempo 120 BPM from the start, 90 BPM from 0:06.5");
  - **no tempo map (BPM 0):** no labels;
  - **no checked stem:** a flat line.
- **Drag:**
  - Pointer-down within 8 px of a change bar (`ew-resize` cursor) starts a drag.
  - While dragging, the bar is drawn at the preview position: `dragTicks` rounds the pointer offset to whole ticks of
    the left section and clamps it to ±64 and to the neighbour limits, using the same formula as Rust. A readout
    next to the waveform shows "+19/64 · 0:06.594".
  - Pointer-up sends `editor_tempo(id, {kind:'move', index, ticks})` once. Rust returns the applied map, which is
    drawn.
  - Escape during a drag cancels it.
  - At full-track zoom one pixel is often several ticks, so dragging is coarse. The keys give exact single steps.
- **Keys** (when the waveform has the focus):
  - The canvas is focusable (`tabindex=0`, `role="slider"`, `aria-valuetext`) when the map has ≥ 2 segments.
  - Focusing it selects change 1, which is drawn brighter and thicker. `↑`/`↓` select the previous/next change.
  - `←`/`→` move it by one 1/64 step, `Shift+←`/`Shift+→` by one beat (16 steps). Each key press is one
    `editor_tempo` call (coalesced: one in flight, the latest target wins). The ±4-beat limit counts from the
    selection (2.6).
  - Space keeps its own meaning there, because `[role="slider"]` is already excluded from the Editor's Space rule.
- **Tempo freshness:** the Editor view calls `editor_tempo(id, {kind:'reload'})` on entry, so the map is fresh after
  BPMs were typed in the Player.
- **The Editor's "BPM & sections" placeholder tab** stays as it is, reserved for `gui-manipulate-backing-track`.
- **Player:**
  - The "Detect" button is removed from the BPM section. `-`, `+`, Reset, the V marker and the field are
    unchanged.
  - The Player's waveform looks exactly as today: it passes no sections.
  - When the Player has the track open, a tempo detected or moved in the Editor reaches its BPM field, V marker,
    metronome and loops through `PlayerEvent::Snapshot` (Q8).

### 2.10 Frontend modules

| File | Change |
|---|---|
| `src/ui/lib/waveform-geometry.ts` | adds `tickMs(bpm)`; `sectionMarks(segments, durationMs, width) -> {x: number \| null /* null = left edge */, bpm, index}[]`; `labelFits(x, nextX, textWidth)`; `dragTicks(startMs, pointerMs, prevBpm, minMs, maxMs) -> number` (rounded, clamped to ±64 and to the neighbour limits). All pure, tested in `waveform-geometry.test.ts` |
| `src/ui/components/player/WaveformView.svelte` | props: `peaks`, `durationMs`, `positionMs`, `loops?`, `currentLoop?`, `segments?`, `onmove?(index, ticks)`, `testid`, `label`. Interaction exists only when `onmove` is given. `PlayerPane` passes `pl.snap` data (no visible change; its tests still pass) |
| `src/ui/lib/editor-state.svelte.ts` | `ed.peaks`, `ed.tempo`, `ed.detect` (`{phase, progress}` or null), `ed.detectMessage`, `ed.detectError`; actions `detect()`, `cancelDetect()`, `moveChange(index, ticks)` (coalesced), `reloadTempo()`; derived `canDetect()`; event cases `peaks`, `detect`, `detected`, `detect-failed`, `detect-cancelled`; log lines |
| `src/ui/lib/ipc.ts` | wrappers `editorDetect`, `editorCancelDetect`, `editorTempo`; the type changes of 2.11 |
| `src/ui/components/editor/MixLane.svelte` | waveform row, Autodetect BPM/Cancel, the phase text, messages |
| `src/ui/views/EditorView.svelte` | `reloadTempo()` on mount when a session is active |
| `src/ui/components/player/BpmSection.svelte` | the Detect button is removed |
| `src/ui/lib/player-state.svelte.ts` | event case `snapshot` → `applySnapshot` (Q8) |

### 2.11 IPC (pattern of `docs/architecture.md`)

```ts
interface TempoSnap { beats_per_bar: number; segments: { start_ms: number; bpm: number }[]; multi: boolean }
interface DetectState { phase: 'checking'|'mixing'|'uploading'|'waiting'|'detecting'|'applying'; progress: number | null }
interface EditorSnapshot { /* existing fields */ peaks: number[]; tempo: TempoSnap; detect: DetectState | null }
type EditorEvent = /* existing */
  | { kind: 'peaks'; id: string; peaks: number[] }
  | ({ kind: 'detect'; id: string } & DetectState)
  | { kind: 'detected'; id: string; tempo: TempoSnap; beats: number; replaced: boolean }
  | { kind: 'detect-failed'; id: string; message: string }
  | { kind: 'detect-cancelled'; id: string };
type TempoEdit = { kind: 'move'; index: number; ticks: number } | { kind: 'reload' };
type PlayerEvent = /* existing */ | { kind: 'snapshot'; id: string; snapshot: PlayerSnapshot };
```

| Command | Returns | Notes |
|---|---|---|
| `editor_detect(id)` | `EditorSnapshot` | async; reads the URL from settings; starts the job; refusals as in 2.8 |
| `editor_cancel_detect(id)` | `()` | |
| `editor_tempo(id, action: TempoEdit)` | `EditorSnapshot` | `move`: `tempo_map::move_change` through `TempoStore::replace`; `reload`: re-reads through the store |

Three new commands, each added to `generate_handler!`, `COMMANDS` in `build.rs` and `capabilities/main.json` (the
static test checks this). `editor_set_stem` gets `Instant::now()` from the IPC layer.

**Log lines:**
- Frontend (`calliope-ui: `):
  - `editor waveform peaks=<count> max=<0..1, 2 decimals>`
  - `editor detect start` / `editor detect cancel`
  - `editor detect phase=<p>`
  - `editor detected segments=<k> bpm=<a,b,..>`
  - `editor detect error message=<m>`
  - `editor tempo move index=<i> ticks=<k> applied=<k'>`
  - `editor tempo reload segments=<k>`
- Rust (`calliope: `):
  - `editor peaks id=<id> gen=<n> ms=<ms>`
  - `editor detect id=<id> phase=<p>`
  - `editor detected …` (2.5)
  - `editor detect failed id=<id> message=<m>` / `editor detect cancelled id=<id>`
  - `tempo saved id=<id> via=<player|file> segments=<k>`

## 3. Tasks

Every task ends with `npm run check` passing (in `src/calliope-gui/`) when it touches the frontend, and `cargo
clippy --workspace --all-targets -- -D warnings` when it touches Rust. GUI test command for one file (builds what
`npm run test:gui` builds, on display `:1`):

```
cd src/calliope-gui && npm run build && cargo build -p calliope-stems && \
DISPLAY=:1 cargo test -p calliope-gui --features e2e-hooks --test <file> -- --ignored --test-threads=1
```

### Task 1: Beats types, validation and client (calliope-lib)
- **files:** `src/calliope-lib/src/stems_api.rs`, `src/calliope-lib/src/stems_client.rs`
- **does:**
  - **Types:**
    - `Health.tasks: Vec<String>` (`#[serde(default)]`; absent = `["stems"]`);
    - `JobStatus.kind: Option<String>` (`skip_serializing_if = "Option::is_none"`, so the stem-job JSON is
      unchanged);
    - `Beats {beats: Vec<f64>, downbeats: Vec<f64>, #[serde(default)] tracker: Option<String>}`;
    - `MAX_BEATS = 20_000`, `MAX_BEATS_JSON_BYTES = 1 << 20`.
  - **`check_beats(&Beats, duration_s) -> Result<(), String>`** with the rules of 2.4.
  - **Client:**
    - `supports_beats(&Health) -> bool`;
    - `submit_beats(file, cancel, on_sent)`: like `submit`, but `POST /v1/beats` and no model query. Refactor the
      shared upload into one private function;
    - `fetch_beats(job, duration_s) -> Result<Beats, ClientError>` (1 MiB cap, `check_beats`; 409 → `Invalid("not
      done")`; 404 → `Restarted`).
  - Unit tests for `check_beats`: every rule, at and over each boundary; downbeat tolerance; empty downbeats are
    allowed. Serde round-trips: the old health JSON without `tasks`, and a stem status without `kind` serialises
    without the key.
- **done when:** `cargo test -p calliope-lib --all-features` and clippy pass.
- **routine:** no

### Task 2: Stub beat tracker
- **files:** `src/calliope-gui/tests/support/stub-beat-tracker` (new, executable, Python 3 stdlib, `#!/usr/bin/env
  python3`), `src/calliope-gui/tests/support/README.md`
- **does:**
  - **Contract:** `stub-beat-tracker <input.flac> <out_dir>`. It reads the duration D from the FLAC STREAMINFO
    (total samples / rate) and writes `<out_dir>/beats.json`. The mode comes from `$STUB_TRACKER_MODE`, else the
    first line of `stub-tracker-mode` in the current directory or two levels up, else `ok`.
  - **Modes:**
    - `ok`: two tempos. 120 BPM beats from 0.5 s for the largest whole number of 4-beat bars that ends at or
      before D/2, then 90 BPM beats from the next 120-BPM beat time until D. Downbeats are every 4th beat from the
      first. For the 16 s "Six Lanes" stems this gives 12 beats at 120 (0.5..6.0 s), then 90 BPM from 6.5 s.
    - `steady`: 120 BPM from 0.5 s to D.
    - `few`: 3 beats.
    - `fail`: message on stderr, exit 3.
    - `slow`: 1 s sleep before each progress line.
    - `hang`: sleeps until killed.
    - `bad-output`: writes `not json`.
  - It prints `progress 0.5` and `progress 1`. `$STUB_TRACKER_LOG` gets the argv appended.
  - The README gets a section in the style of the stub-separator section.
- **done when:** `python3 -I tests/support/stub-beat-tracker tests/fixtures/import/stems/drums.flac <tmp>` writes
  valid JSON with 12 + 15 beats (16 s file: 0.5..6.0 s at 120, 6.5..15.83 s at 90), checked by Task 3's unit test. `bash -c 'STUB_TRACKER_MODE=fail
  tests/support/stub-beat-tracker x y; test $? -eq 3'`.
- **routine:** no

### Task 3: calliope-stems: beat jobs, routes, flags
- **files:** `src/calliope-stems/src/{cli,config,jobs,server,main}.rs`
- **does:**
  - **Flags:**
    - `--beat-tracker PATH` (optional; must be an executable file, like `--separator`);
    - `--beat-timeout-min N` (default 10);
    - hidden `--beat-timeout-ms N` (minimum 100; it wins over minutes in any order, like
      `--separator-timeout-ms`);
    - help text and `cli` unit tests.
  - **Jobs:**
    - `Job.kind` (`Stems | Beats`) and `Job.beats: Option<Beats>`.
    - The worker runs the tracker for beats jobs: argv `[input.flac, out_dir]`, same supervision, timeout from the
      beat timeout, progress relay, stderr tail. On exit 0 it reads `out/beats.json` (≤ 1 MiB) and runs
      `check_beats` with the upload's duration. It skips `measure_stems`.
    - The status shows `kind: Some("beats")` for beats jobs.
  - **Server:**
    - routes `POST /v1/beats` (shared upload path; 501 without a tracker) and `GET /v1/jobs/<id>/beats` (200/409/404);
    - `stems` on a beats job → 404;
    - `tasks` in health;
    - `route_table` tests extended (including `/v1/beats/`, `/v1/jobs/<id>/beats/x` → 404).
  - Log lines as in 2.4.
- **done when:** `cargo build -p calliope-stems && cargo test -p calliope-stems --bins` passes. `calliope-stems
  --help` lists `--beat-tracker` and `--beat-timeout-min`, but not the `-ms` flag.
- **routine:** no

### Task 4: Protocol conformance and server README for beats
- **files:** `src/calliope-stems/tests/conformance.rs`, `src/calliope-stems/tests/smoke.rs`,
  `src/calliope-stems/README.md`
- **does:**
  - **Conformance:** extend the suite (raw HTTP and the shared client) with the real binary on `127.0.0.1:0`, plus
    `--beat-tracker ../calliope-gui/tests/support/stub-beat-tracker` and `--beat-timeout-ms 500`. Cover:
    - health `tasks` with and without the tracker;
    - `POST /v1/beats` → 202, then done; `GET …/beats` matches the stub's numbers; a stem fetch on it → 404;
      `beats` on a stem job → 404; 409 while running (`slow`);
    - 501 without a tracker; the same 411/413/415 cases as jobs;
    - the stub modes `fail` (error text contains the stub message), `bad-output` and `few`;
    - `hang` + `DELETE` (process group gone, folder removed) and `hang` + timeout (failed within about 1 s);
    - a beats job queued behind a `slow` separation, then running;
    - the stem-job status JSON without a `kind` key.
  - Reuse the existing helpers. Add cases to the existing tests where they fit instead of duplicating server start-up
    code.
  - **README:** API section (the endpoints and the beats document), "The beat tracker contract", the flags table,
    and the deployment steps of section 5 of this plan.
- **done when:** `cargo build -p calliope-stems && cargo test -p calliope-stems` passes in under 60 s (no real
  timeouts: only the `-ms` knobs and the `slow`/`hang` modes).
- **routine:** no

### Task 5: Real tracker adapter, unit example, licence records
- **files:** `src/calliope-stems/trackers/beat-this.sh` (new), `src/calliope-stems/deploy/calliope-stems.service`,
  `src/calliope-stems/tests/smoke.rs` (syntax test), `docs/licences.md`
- **does:**
  - **The adapter** as in 2.4:
    - `set -euo pipefail` and argument checks;
    - `BEATS_HOME`, `PY` and `CKPT` checks with clear messages (exit 2);
    - the embedded Python program: soundfile read, mono, device choice with CPU fallback on
      `torch.cuda.OutOfMemoryError`, `Audio2Beats`, atomic JSON write, progress lines, `tracker` = `beat_this
      <version> final0 <device>`.
  - **The unit example:** `Environment=BEATS_HOME=%h/edge-ai/beats` and `--beat-tracker
    %h/.local/bin/calliope-beats-beat-this` in `ExecStart`.
  - **A test:** `bash -n` on the adapter, plus the heredoc extracted and parsed with `python3 -I -c "import
    ast,sys; ast.parse(sys.stdin.read())"`. It does not import `beat_this`.
  - **`docs/licences.md`, server table:**
    - beat-this 1.1.0 (MIT, code and weights);
    - the `final0` checkpoint (MIT; training-data note);
    - torch 2.11.0 (BSD-3-Clause) and torchaudio 2.11.0 (BSD-2-Clause, per its package classifier "BSD License");
    - einops (MIT), rotary-embedding-torch (MIT), soxr (LGPL-2.1-or-later), numpy (BSD-3-Clause) and soundfile
      0.14 (BSD-3-Clause; its wheel bundles libsndfile, LGPL-2.1+).

    All of them run as separate processes on the owner's server and are never distributed. Rows not read from
    local metadata are marked "per PyPI, 2026-10-11; confirm against the installed `dist-info`".
  - The sentence "The server's Rust code only runs `audio-separator` through the adapter" gains the tracker
    adapter.
- **done when:** `cargo test -p calliope-stems --test smoke` passes; `cargo test -p calliope-gui --test frontend`
  (licence record test) passes.
- **routine:** no

### Task 6: `tempo_map::move_change`
- **files:** `src/calliope-gui/src/tempo_map.rs`
- **does:** `pub(crate) const MAX_MOVE_TICKS: i64 = 64`; `pub(crate) fn tick_ms` (make the existing helper
  crate-visible); `move_change` as in 2.6. Unit tests:
  - a move by +1 and -1 lands exactly on the left grid (`(start_i - start_{i-1}) / tick_ms(bpm_{i-1})` is an integer
    within 1e-9);
  - ±64 accepted, ±65 refused, index 0 refused, index out of range refused;
  - clamping at the left neighbour + 1 beat and at the right neighbour - 1 beat, returning the applied count;
  - a 3-segment map where moving change 1 leaves change 2 within half a tick of its old time and on change 1's new
    grid;
  - loops through `PlayerState::apply_tempo_change` keep their ms within half a tick.
- **done when:** `cargo test -p calliope-gui --lib tempo_map` passes.
- **routine:** no

### Task 7: Beats to tempo sections (`tempo_detect.rs`)
- **files:** `src/calliope-gui/src/tempo_detect.rs` (new), `src/calliope-gui/src/lib.rs`
- **does:** `detect(...)` as in 2.5, plus the summary. Unit tests on synthetic beat lists (seeded jitter from a
  tiny inline LCG, no new crate):
  - steady 120 BPM, with and without ±10 ms jitter → one segment, 120.0, anchor at the first downbeat ±5 ms;
  - the stub's `ok` pattern (12 beats at 120 from 0.5 s, then 90) → `[(500, 120.0), (6500, 90.0)]` exactly;
  - 120 → 90 → 140 → three segments, each change on the previous grid (integer tick distance);
  - a 4-beat fill at another tempo → no change;
  - a half-time track at 70 BPM over 120 BPM music stays 70 (no guessing); 350 BPM folds to 175;
  - no downbeats → anchor at the first beat;
  - unsorted or duplicate input; fewer than 8 beats → `Err` with the exact message;
  - drift: 121.04 BPM for 4 minutes → bpm 121.0 and every beat within 60 ms of the grid.
- **done when:** `cargo test -p calliope-gui --lib tempo_detect` passes.
- **routine:** no

### Task 8: Mix peaks and debounce (`mix_waveform.rs`)
- **files:** `src/calliope-gui/src/mix_waveform.rs` (new), `src/calliope-gui/src/lib.rs`
- **does:** `mix_peaks` and `Debounce` as in 2.7, plus `pub const DELAY: Duration = 300 ms`. Unit tests:
  - peaks of one stem at 0 dB equal `player_source::peaks` of the same PCM (same buckets);
  - two stems summed, -6 dB, mono→stereo, clipping at 1.0;
  - all gains 0 → zeros without iterating (`cancel` already set still gives `Some(zeros)`);
  - cancel mid-way → `None`;
  - Debounce: `touch(t0)`; `take_due(t0+299)` false, `take_due(t0+300)` true and only once; `touch(t0+200)`
    moves the deadline to t0+500.
- **done when:** `cargo test -p calliope-gui --lib mix_waveform` passes.
- **routine:** no

### Task 9: Editor: waveform scheduling, peaks event, snapshot
- **files:** `src/calliope-gui/src/editor.rs`, `src/calliope-gui/src/editor_tests.rs`, `src/calliope-gui/src/ipc.rs`
  (pass `Instant::now()` to `set_stem`), `src/calliope-gui/src/editor_ipc_tests.rs` (call sites)
- **does:** 2.7 in the manager:
  - `Debounce` in the session, `touch` in `set_stem(.., now)` and at `finish_open` (due at once);
  - in `tick`: start the worker, install the result if its generation is current, emit `Peaks`;
  - cancel on close and switch;
  - `peaks` in `EditorSnapshot`;
  - `set_peaks_fn` test hook.

  Tests (ManualBackend, fixture copies, injected `Instant`s):
  - changes at t0 and t0+200, ticks at t0+499 → no computation started; t0+500 → exactly one;
  - a computation held by a blocking hook (barrier) while `set_stem`, `play`, `seek` and `snapshot` all return,
    and audio is pulled from the ManualBackend;
  - a new change during a running computation cancels it, and its late result is never emitted;
  - the emitted peaks equal `mix_peaks` of the checked stems;
  - close cancels the worker.
- **done when:** `cargo test -p calliope-gui --lib editor` passes.
- **routine:** no

### Task 10: Tempo store, Player hand-over, Editor tempo
- **files:** `src/calliope-gui/src/tempo_store.rs` (new), `src/calliope-gui/src/lib.rs`, `src/calliope-gui/src/player.rs`,
  `src/calliope-gui/src/player_tests.rs`, `src/calliope-gui/src/player_ipc.rs`, `src/calliope-gui/src/editor.rs`,
  `src/calliope-gui/src/editor_tests.rs`
- **does:**
  - **`TempoStore`:**
    `trait TempoStore: Send + Sync { fn load(&self, id, dir) -> Result<TempoMap, String>; fn replace(&self, id, dir,
    f: &mut dyn FnMut(&TempoMap) -> Result<TempoMap, String>) -> Result<TempoMap, String> }`.
  - **`FileTempoStore`:** `player_state::load`, refuse read-only, `apply_tempo_change`, `speed = 1.0`, `save`.
  - **The Player:**
    - `PlayerManager::replace_tempo(id, map, now)` does what `set_bpm` does with a whole map, and emits
      `PlayerEvent::Snapshot`;
    - `PlayerTempoStore(Arc<PlayerHost>…)` holds the host lock across "is open? → session : file";
    - logs `tempo saved id=… via=…`.
  - **The Editor:** `OpenSpec.tempo` (loaded through the store by `do_open_editor`), `Session.tempo`, `tempo` in the
    snapshot, `set_tempo(id, map)`.
  - **Tests:**
    - file store: loops re-snapped, unknown fields kept, a newer schema refused and unchanged on disk, a missing
      folder not created;
    - Player: `replace_tempo` on an open session updates grid, loops, snapshot and the debounced write, emits
      `Snapshot`, and `player.json` on disk has the new map after the flush;
    - player store: an open session goes through the session, a closed one through the file.
- **done when:** `cargo test -p calliope-gui --lib tempo_store player editor` passes (one run per filter).
- **routine:** no

### Task 11: Detection job (`beat_detect.rs`) and manager integration
- **files:** `src/calliope-gui/src/beat_detect.rs` (new), `src/calliope-gui/src/beat_detect_tests.rs` (new),
  `src/calliope-gui/src/lib.rs`, `src/calliope-gui/src/editor.rs`
- **does:** 2.8: `DetectParams`, `DetectShared`, the job thread with its phases, the part-file cleanup in every path,
  the manager's `start_detect` / `cancel_detect` / outcome in `tick` / cancel on close and on `shutdown_detect(cap)`,
  and the events.

  Tests against the REAL `calliope-stems` (`target/debug/calliope-stems`, `--listen 127.0.0.1:0`, stub separator,
  stub tracker, `--beat-timeout-ms 1000`, poll 50 ms), the way `import_job_tests.rs` starts it (reuse its helper by
  moving it to a shared test-support module if needed):
  - `ok` → `Detected`, `player.json` has `[(500,120),(6500,90)]` (Six Lanes), no `.calliope-beats-*` left, phases in
    order;
  - `steady` → one segment;
  - `few` / `fail` / `bad-output` → `DetectFailed` with the exact messages, `player.json` unchanged;
  - server without `--beat-tracker` → the "cannot detect beats" message;
  - URL with nothing listening → "Cannot reach…";
  - `hang` + `cancel_detect` → `DetectCancelled`, the server job deleted (status 404), the stub process gone, the part
    file gone;
  - `slow` + track close → cancelled;
  - a second `start_detect` while running → refused;
  - no checked stem → refused.
- **done when:** `cargo build -p calliope-stems && cargo test -p calliope-gui --lib beat_detect` passes in under 30 s.
- **routine:** no

### Task 12: IPC commands and TS wrappers
- **files:** `src/calliope-gui/src/ipc.rs`, `src/calliope-gui/src/gui.rs`, `src/calliope-gui/build.rs`,
  `src/calliope-gui/capabilities/main.json`, `src/calliope-gui/src/editor_ipc_tests.rs`, `src/ui/lib/ipc.ts`,
  `src/ui/lib/ipc-editor.test.ts`, `src/ui/lib/ipc-player.test.ts`
- **does:**
  - **Commands:** `editor_detect`, `editor_cancel_detect` and `editor_tempo` (2.11).
    - `do_*` functions take the store as `Arc<dyn TempoStore>`, so `editor_ipc_tests` use `FileTempoStore`.
    - `RunEvent::Exit` cancels a detection (2 s cap).
    - The app's `PlayerTempoStore` is created in `setup`.
  - **TS:** the types and wrappers of 2.11 (including `PlayerEvent` `snapshot`), with mockIPC tests for argument
    names.
  - **`editor_ipc_tests`:** `move` and `reload` through the IPC functions; `editor_detect` refusals without a URL
    and without a checked stem.
- **done when:** `cargo test -p calliope-gui --lib editor_ipc` and `cargo test -p calliope-gui --test frontend`
  (command lists match) pass; `npx vitest run src/ui/lib/ipc-editor.test.ts src/ui/lib/ipc-player.test.ts` passes.
- **routine:** no

### Task 13: Remove the Player's Detect button
- **files:** `src/ui/components/player/BpmSection.svelte`, `src/ui/components/player/ControlBar.test.ts`,
  `src/ui/acceptance_player.test.ts`
- **does:** exact edits:
  1. In `BpmSection.svelte` delete the line
     `<Button variant="outline" class="h-[3.25rem] text-lg" disabled title={HINT}>Detect</Button>`.
  2. In `ControlBar.test.ts` line 107 change `['Slower', 'Faster', 'Reset', 'Detect']` to `['Slower', 'Faster',
     'Reset']`, and directly after that line add
     `expect(screen.queryByRole('button', { name: 'Detect' })).toBeNull();` (use the file's existing
     `screen` import; if the file names it differently, use that name).
  3. In `src/ui/acceptance_player.test.ts` change line 119 `expect(btn('BPM', 'Detect').disabled).toBe(true);` to
     `expect(screen.queryByRole('button', { name: 'Detect' })).toBeNull();`. On line 129 remove `, 'Detect'` from
     the list, and on line 122 change `-, +, Reset and Detect stay disabled` to `-, +, Reset stay disabled`.
- **done when:** `cd src/calliope-gui && npx vitest run src/ui/components/player/ControlBar.test.ts
  src/ui/acceptance_player.test.ts && npm run check` passes.
- **routine:** yes

### Task 14: Waveform geometry for sections and drag
- **files:** `src/ui/lib/waveform-geometry.ts`, `src/ui/lib/waveform-geometry.test.ts`
- **does:** `tickMs`, `sectionMarks`, `labelFits` and `dragTicks` (2.10). `dragTicks` mirrors `move_change`'s clamp
  (±64, one beat from each neighbour). Tests use the same numbers as Task 6, so frontend and Rust agree.
- **done when:** `npx vitest run src/ui/lib/waveform-geometry.test.ts` passes.
- **routine:** no

### Task 15: Shared WaveformView with sections, drag and keys
- **files:** `src/ui/components/player/WaveformView.svelte`, `src/ui/components/player/WaveformView.test.ts`,
  `src/ui/components/player/PlayerPane.svelte`
- **does:**
  - **Props** as in 2.10; `PlayerPane` passes `pl.snap`/`pl.transport` data, and its `data-testid` stays
    `player-waveform`.
  - **Drawing:** section bars and labels (2.9), plus the drag preview and readout.
  - **Pointer and keys:** pointer handlers and the keys of 2.9, only when `onmove` is set.
  - **Tests:**
    - a Player-style render (no segments) draws exactly what it does today: the existing tests are kept;
    - segments produce the bar x-positions (canvas mock records `fillRect`) and the `aria-label` text;
    - a drag of N px calls `onmove(1, expectedTicks)` once;
    - Escape cancels a drag;
    - keys: `→` = +1, `Shift+→` = +16, and the 64-tick budget stops further calls;
    - `↑`/`↓` change the selection.
- **done when:** `npx vitest run src/ui/components/player/` and `npm run check` pass.
- **routine:** no

### Task 16: Editor frontend state for waveform, tempo and detection
- **files:** `src/ui/lib/editor-state.svelte.ts`, `src/ui/lib/editor-state.test.ts`, `src/ui/lib/player-state.svelte.ts`,
  `src/ui/lib/player-state.test.ts`
- **does:**
  - **Editor state:** the state, actions, `canDetect()` (needs the settings' `edge_ai_url`, from the existing
    settings state) and the event cases of 2.10. Messages: the newest of save and detect wins the slot. Log lines
    as in 2.11.
  - **Player state:** the event case `snapshot` → `applySnapshot` (Q8).
  - **Tests (mockIPC):**
    - `canDetect` truth table (no stem, a stem at Off, no URL, running);
    - the event sequence `detect` → `detected` updates `ed.tempo` and the message text;
    - `detect-failed` and `detect-cancelled`;
    - `peaks` replaces `ed.peaks`;
    - `moveChange` coalescing (one call in flight);
    - stale-id events are ignored;
    - a Player `snapshot` event updates `pl.snap.tempo`.
- **done when:** `npx vitest run src/ui/lib/editor-state.test.ts src/ui/lib/player-state.test.ts` passes.
- **routine:** no

### Task 17: Mix lane UI
- **files:** `src/ui/components/editor/MixLane.svelte`, `src/ui/components/editor/EditorPane.test.ts`,
  `src/ui/views/EditorView.svelte`
- **does:** the layout of 2.9:
  - the `WaveformView` with `onmove={moveChange}` and `data-testid="mix-waveform"`;
  - the Autodetect BPM button / Cancel, its hints, the phase text;
  - the messages in the reserved slot;
  - `reloadTempo()` on view mount when a session is active.

  `EditorPane.test.ts` covers:
  - the button states and hints;
  - the click → `editor_detect`;
  - the Cancel → `editor_cancel_detect`;
  - the phase text;
  - the success and error messages;
  - the waveform present in the Mix lane.
- **done when:** `npx vitest run src/ui/components/editor/` and `npm run check` pass.
- **routine:** no

### Task 18: GUI end-to-end, visual review, docs
- **files:** `src/calliope-gui/tests/gui_editor_e2e.rs`, `src/calliope-gui/tests/common/mod.rs`, `docs/ui.md` ("Decided by
  the team"), `src/calliope-gui/README.md`
- **does:**
  - **Helper:** in `common`, pass `--beat-tracker <stub-beat-tracker> --beat-timeout-ms 3000` in
    `start_import_app`'s server script (harmless for the import tests), and add `start_detect_app(dirs,
    tracker_mode)`: a copy of `library-editor` as the repository, the server and stub tracker under `unshare -rn`.
    Factor out the shared part rather than copying it.
  - **Two tests added to `gui_editor_e2e.rs`:**
    1. `mix_waveform_and_autodetect` (Six Lanes, `ok`):
       - check a stem: within 1 s an `editor waveform` line with `max>0` appears;
       - check and uncheck three stems quickly: exactly one new `editor peaks` line, ≥ 300 ms after the last change
         (Rust log timestamps from the test's clock), and a `play` meanwhile is logged at once;
       - uncheck all: Autodetect is disabled and `max=0.00`;
       - Autodetect: the phase lines, then `editor detected segments=2 bpm=120,90`; `player.json` has the two
         segments;
       - screenshot `editor-detected-<theme>.png` in both themes;
       - focus the waveform: `Shift+→` → `applied=16`, `→` → `applied=1`; `player.json` start of change 1 =
         6500 + 17 × 31.25 ms;
       - drag the bar 40 px with xdotool → `editor tempo move` logged, `|applied| ≤ 64`;
       - switch to the Player (Alt+5): the BPM field shows 120, the V marker is checked, and there is no Detect
         button (screenshot).
    2. `autodetect_errors_and_cancel`:
       - `fail` → the message in the slot;
       - `hang` → Cancel → `editor detect cancelled`, no stub process left;
       - no edge-AI URL (settings without it) → button disabled with its hint.
  - **Visual review** at 1280x800 and 1024x640, both themes:
    - the waveform height;
    - the labels legible from 1-2 m;
    - the bar contrast;
    - nothing moving during detection;
    - the lanes still reachable at 1024x640 (record the known limit in `docs/ui.md` if fewer lanes fit).
  - **Docs:** the `docs/ui.md` decisions (Mix lane order, labels, drag and keys); `README.md` usage (Autodetect BPM,
    and that the server needs the beat tracker).
- **done when:** the GUI command above with `--test gui_editor_e2e` passes. The screenshots in `target/gui-shots/`
  were reviewed. `npm test` (headless suite) passes.
- **routine:** no

Order: 1 → 2 → 3 → 4 → 5 (server side, independent of the GUI); 6 → 7 → 8 → 9 → 10 → 11 → 12; 13 any time; 14 → 15 →
16 → 17 → 18. Tasks 6-8 and 13-14 have no dependencies on the server tasks.

## 4. Test strategy

- **Pure Rust units:**
  - `tempo_map::move_change` (Task 6);
  - `tempo_detect` on synthetic beat lists, with no audio and no AI (Task 7);
  - `mix_waveform` (Task 8);
  - `check_beats` and the serde compatibility (Task 1);
  - the cli flags (Task 3).
- **Editor timing** (AC "0.3 s", "restart the wait", "without blocking the GUI"): injected `Instant`s on
  `EditorManager::tick`/`set_stem`, and a blocking peaks hook (barrier) proving that commands and the audio callback
  run while a computation is in progress (Task 9). The GUI test only confirms it end to end with log timing, never
  with long sleeps.
- **Edge-AI server:**
  - The real `calliope-stems` binary with `tests/support/stub-separator` and the new
    `tests/support/stub-beat-tracker`, on `127.0.0.1:0` (Rust tests), or inside `unshare -rn` on `127.0.0.1:8765`
    (GUI tests).
  - Timeouts use `--beat-timeout-ms`. Polling runs at 50 ms in tests.
  - An "old server" is the binary without `--beat-tracker` (`tasks: ["stems"]`).
  - No test runs Beat This!, PyTorch or the real adapter, which is only syntax-checked. No test touches archserver
    or the `calliope-stems` listening on this laptop's `0.0.0.0:8765` (Rust tests bind port 0; GUI tests live in
    their own network namespace).
- **Persistence:** `player.json` on temp copies: file store and Player session hand-over (Task 10), then on disk
  after detection and moves (Tasks 11, 18).
- **Audio:** none of this plays audio. Existing rule: ManualBackend in units, NullBackend in GUI builds.
- **Frontend:** vitest for geometry, the shared waveform component (canvas mocked), the editor state and the Mix
  lane; `svelte-check`.
- **GUI on a display:** two new tests in `tests/gui_editor_e2e.rs` (Task 18), screenshots for the visual review.
  Player: no new GUI test. Detect's absence is checked by vitest (Task 13) and in Task 18's Player screenshot.
- **Whole suite:** `cd src/calliope-gui && npm test` (headless: svelte-check, vitest, vite build, `cargo test
  --workspace`, clippy), then `npm run test:gui` (all GUI binaries, private displays).

## 5. Deployment

| Component | Runs on | How it gets there |
|---|---|---|
| calliope-gui (all GUI changes) | laptop | `cd src/calliope-gui && npm run build:app` → `target/release/calliope-gui` |
| calliope-stems (beats endpoints) | archserver | rebuilt and restarted by the owner (below) |
| `trackers/beat-this.sh` adapter + Beat This! venv + checkpoint | archserver | installed by the owner (below) |

**archserver steps (owner only; agents never deploy):**
1. Update the checkout and build: `git pull && cargo build --release -p calliope-stems`.
2. Create the Beat This! environment:
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
4. Smoke-test the adapter on any FLAC:
   `BEATS_HOME=~/edge-ai/beats calliope-beats-beat-this song.flac /tmp/bt && head -c 300 /tmp/bt/beats.json`. It
   prints progress lines, and the JSON has `beats`, `downbeats` and `tracker` (the device is named in `tracker`).
5. Edit `~/.config/systemd/user/calliope-stems.service` as in the updated example unit:
   `Environment=BEATS_HOME=%h/edge-ai/beats` and `--beat-tracker %h/.local/bin/calliope-beats-beat-this` in
   `ExecStart`. Then:
   `systemctl --user daemon-reload && systemctl --user restart calliope-stems`.
6. Check from the laptop: `curl http://archserver:8765/v1/health` shows `"tasks":["stems","beats"]`.
7. Optional: confirm the licences in `~/edge-ai/beats/.venv/lib/python3.12/site-packages/*.dist-info/METADATA`
   against `docs/licences.md`.

## 6. Manual checks (owner: laptop + archserver)

- **M1:**
  1. On a real stem track in the Editor, check drums + bass + vocals: the waveform appears within about 1 s and
     looks like the song.
  2. Uncheck the drums: the waveform changes about 0.3 s after you stop clicking, and playback never stutters
     meanwhile.
- **M2:**
  1. With a song at a steady tempo, press Autodetect BPM: the status goes Preparing → Uploading → Detecting → Saving,
     and "Detected 1 tempo: N BPM" appears.
  2. In the Player, Metronome On + Play: the clicks are on the beat from the first downbeat to the end, with no
     audible drift.
- **M3:**
  1. Run Autodetect BPM on a song with a known tempo change: an amber bar appears near the change with the new BPM.
  2. In the Player, the V marker is checked and the metronome follows the change.
- **M4:**
  1. Drag the change bar a little: it moves in small steps and stops at 4 beats. Click the waveform and use
     ←/→ and Shift+←/→: one tick, one beat.
  2. In the Player, the metronome's change point moved accordingly.
- **M5:**
  1. Start Autodetect BPM, then press Cancel at once: "cancelled". On archserver, `journalctl --user -u
     calliope-stems` shows the job cancelled, and no `python` process is left.
  2. With the server stopped, the message says it can't reach the server.
- **M6 (speed):**
  1. Note the detection time for a 4-minute song.
  2. Optionally, run `nvidia-smi` during detection to confirm the GPU (or the CPU fallback when Ollama fills the
     card).
- **M7:** In the Player, the BPM section has no Detect button. Loops set before a detection still sit on the same
  music afterwards.

## 7. Acceptance mapping (refined criteria, for the owner's confirmation)

The spec's criteria are loose. The refined Given/When/Then below are what the tester will check. Spec criteria are
marked S1-S7, in the spec's order.

| # | Refined criterion | Spec | Tasks | Tests |
|---|---|---|---|---|
| A1 | Given an active stem track in the Editor, then the Mix lane shows a full-width waveform of the current mix (checked stems at their volumes) with a playhead | req 1 | 9, 15, 17 | editor_tests (peaks = mix_peaks), EditorPane.test, gui `mix_waveform_and_autodetect` |
| A2 | Given a stem's checkbox or volume changes, when 0.3 s pass without another change, then the waveform is recomputed once and redrawn; meanwhile Play/Pause, seeking and the other lanes respond at once and the audio does not stop | S1 | 8, 9, 18 | mix_waveform Debounce, editor_tests (barrier hook), gui (log timing) |
| A3 | Given a change, when another change comes within 0.3 s, then the wait restarts from the last change and only one recomputation happens | S2 | 8, 9 | editor_tests (t0, t0+200 → due t0+500), gui |
| A4 | Given no stem is checked (or all checked stems are Off), then the waveform is flat and Autodetect BPM is disabled with a hint; given at least one checked stem above Off and an edge-AI server set, then it is enabled | S3, req 2 | 16, 17 | editor-state `canDetect`, EditorPane.test, gui |
| A5 | Given Autodetect BPM is clicked, then the current mix is rendered and sent to calliope-stems (`POST /v1/beats`); the phases and a Cancel button are shown; playback keeps working | S4 | 1, 3, 11, 17 | conformance, beat_detect_tests, gui |
| A6 | Given a detection is running, when Cancel is pressed (or the track is switched, or the app closes), then the server job is deleted, no file changes, and the button is available again | (new) | 11, 12, 17 | beat_detect_tests (hang + cancel, close), gui `autodetect_errors_and_cancel` |
| A7 | Given the server is unreachable, has no beat detection, fails, or finds fewer than 8 beats, then an inline message says so and the tempo map is unchanged | (new) | 11, 16 | beat_detect_tests, editor-state.test, gui (`fail`) |
| A8 | Given a steady song, when detection succeeds, then the track has one tempo starting at the first downbeat, and the waveform shows "N BPM" at the left edge with no bar | req 3 | 7, 11, 15 | tempo_detect (steady), beat_detect_tests (`steady`), WaveformView.test |
| A9 | Given a song with several tempos, when detection succeeds, then each change starts on the nearest 1/64 tick of the previous section's grid, and the waveform shows an amber bar at each change followed by its BPM | S5, req 3-4 | 7, 15 | tempo_detect (two/three tempos), WaveformView.test, gui (`bpm=120,90`) |
| A10 | Given detection succeeded, then within 1 s `player.json` holds the new map (loops keep their time, speed 100 %), the previous map is replaced, and after a restart the Player and the Editor show it | S6 | 10, 11 | tempo_store tests, beat_detect_tests, gui |
| A11 | Given the Player has the track open, when a tempo is detected or moved in the Editor, then the Player's BPM field, V marker, metronome and loops follow without reopening the track | (Q8) | 10, 16 | player_tests (`replace_tempo`), player-state.test, gui (Alt+5 check) |
| A12 | Given several tempos, when the user drags a change bar, then it moves in 1/64 steps of the left section's tempo, at most 4 beats from where the drag started, never within one beat of its neighbours; the result is saved on release and later changes stay at their time | S7, req 4 | 6, 14, 15, 10 | tempo_map `move_change`, waveform-geometry `dragTicks`, WaveformView.test, gui (drag) |
| A13 | Given the waveform has the focus, when ↑/↓ select a change and ←/→ (Shift: one beat) are pressed, then the same rules as A12 apply (keyboard alternative) | req 4 | 15, 18 | WaveformView.test, gui (keys) |
| A14 | Given the Player view, then the BPM section has no Detect button, and nothing else in the Player changes | out of scope | 13, 15 | ControlBar.test, acceptance_player.test, existing Player GUI tests unchanged |
| A15 | Given a BPM was typed in the Player, when the user returns to the Editor, then the waveform shows the current sections | (new) | 10, 17 | editor_ipc_tests (`reload`), EditorPane.test |

## 8. Open questions (each with a recommendation)

1. **Where the detected tempo is stored.**
   - **Recommendation:** the track's one tempo map in `player.json`, shared with the Player (2.2), written within 1 s.
     This is how the metronome and count-in use it.
   - **Alternatives:**
     - (b) `track.json` (the overview's "track metadata"): the Player would have to move its map there, which is a
       Player change, and every BPM typed would change `track.json`'s revision (Library save conflicts);
     - (c) a copy per backing variant: it would go stale against the Player's map.
   - The architecture already expects the map to move into `track.json` with `gui-manipulate-backing-track`, if you
     want that then.
   - If (b), Tasks 10-12 change (the store writes `track.json` through the repository lock).
2. **What is analysed.**
   - **Recommendation:** the current Editor mix (checked stems at their volumes), rendered on the fly. The user can
     include the drums.
   - **Alternatives:** the saved backing (needs a Save first), `original.flac` (often missing), or the drums stem
     only.
3. **Method and server.**
   - **Recommendation:** Beat This! 1.1.0 (MIT code and weights) on archserver, behind additive `calliope-stems`
     endpoints and a tracker adapter, in its own uv venv (about 4 GB disk), with the checkpoint pre-downloaded. It
     uses the GPU, falling back to the CPU.
   - **Alternative:** a local non-AI detector in Rust (no server, no deployment, but weaker on real music and tempo
     changes).
   - Please also confirm that the training-data note (some training sets are copyrighted or under limited CC
     licences; the weights are MIT) is fine for personal use.
4. **Step size.**
   - "1/64 of the BPM" is read as the existing Player grid, a 1/64 note (1/16 of a beat), so 4 beats = 64 steps.
   - If you meant 1/64 of a beat, the Player's grid would change too.
   - **Recommendation:** keep the 1/64 note.
5. **What "no more than 4 beats" is measured from.**
   - **Recommendation:** from where the bar is when a drag starts (or when it is selected with the keyboard). A badly
     detected change can then still be fixed in several moves.
   - **Alternative:** at most 4 beats from the detected position, for good. That needs the detected positions stored
     in `player.json`.
6. **Section sensitivity.**
   - **Recommendation:** a change is a tempo difference over 4 % that lasts at least 8 beats. Sections are at least 2
     bars, and each BPM is rounded to 0.1.
   - Gradual accelerandos and ritardandos are not followed (they stay in one section).
   - The time signature stays 4/4 (your earlier decision). The detected bar length is only logged.
7. **"The mix has at least a stem in it".**
   - **Recommendation:** at least one checked stem whose volume is above Off. This differs from Save, which only
     needs a checked stem, because a silent mix can't be analysed.
   - The button also needs the edge-AI server to be set in Settings.
8. **Player changes beyond removing Detect** (please confirm):
   - (a) **Needed for the feature:** when the Player has the track open, the detected or moved tempo is pushed into
     it (an internal event, no new control). Otherwise the Player would show the old tempo, and could overwrite the
     new one on its next save. **Recommendation: yes** (Tasks 10, 16).
   - (b) **Invisible refactor:** the Player's waveform component becomes shared, with props. It looks and behaves
     the same (Task 15). **Recommendation: yes.**
   - (c) **Not planned:** showing the tempo sections in the Player's waveform too. **Recommendation:** a small later
     change, if you want it.
9. **Replacing an existing tempo map.**
   - **Recommendation:** Autodetect replaces the whole map (manual or earlier detection) without a confirmation
     dialog (no pop-ups). The result line says that it replaced the map. There is no undo. Loops keep their place in
     the music.
   - **Alternative:** a Yes/No confirmation when a map exists.
10. **Small UI choices:**
    - the labels read "120 BPM" (shortened to "120" when space is short);
    - the change bars are amber, like the loop marks;
    - the Editor's "BPM & sections" placeholder tab stays for `gui-manipulate-backing-track`.
    - **Recommendation:** as listed.
11. **Acceptance criteria:** please confirm the refined criteria A1-A15 in section 7. A6, A7, A11, A13 and A15 are
    new: they cover cancel, errors, the Player hand-over, the keyboard alternative and the tempo refresh.
