# Plan: Slow count-in and lead silence in exports

Spec: `specs/gui-export-slow-count-in.md`. Base: `882b763` on branch `spec/gui-export-slow-count-in`. Predecessors:
`specs/gui-export.plan.md` (count-in, pipeline, prepare/start), `specs/gui-export-info.plan.md` (`track.json`
`exports`, revision without `exports`), `specs/gui-first-beat.plan.md` (`skip_frames`),
`specs/gui-export-rc5-quick.plan.md` (pane layout, GUI test coordinates). The spec's open questions were answered by the
owner on 2026-10-30 (the proposed defaults): silence on every export, silence length = the count-in length, the switch
remembered per track in the track's own data (off for existing tracks), all three targets.

## 1. Summary

Every export (RC-5, MP3, OGG) now starts with silence as long as its count-in, then the count-in, then the track. A new
"Slow count-in (half tempo)" switch in the Export pane doubles the spacing of the six ticks (ticks at beats 0, 2, 4, 5,
6, 7 of the track's tempo, first beat at beat 8) and so also doubles the silence. The switch is stored per track in
`track.json` (`exports.slow_count_in`), and the count-in line says what will be written and how much it adds. The
Player's count-in is not touched.

## 2. Design

### 2.1 What is reused, what is new

| Area | Reused | New / changed |
|------|--------|---------------|
| Storage | `track.json` `exports` object (kept in `TrackMeta.extra`, ignored by `revision`, written at the JSON level by `write_raw`) | key `exports.slow_count_in` (2.2); `export_record::{slow_count_in, set_slow_count_in}`; `TrackRecord.slow_count_in`; `Repository::set_slow_count_in` |
| Audio | `count_in_plan`, `run` (writes `lead_frames` zeros, then the original; mixes `ticks` at absolute frames), `LEAD_BEATS` / `TICK_BEATS` from `metronome.rs` | `count_in_plan(map, rate, slow)`; `CountInPlan.{slow, silence_frames, count_in_frames}`; `lead_frames` = silence + count-in (2.3). `run` is unchanged |
| Job | `ExportState::start` recomputes the count-in from the tempo map as it is now | `start(token, overwrite, tempo, slow, sink, on_done)`; the `export start` log line gains `slow=` and `silence_frames=` |
| IPC | `do_export_info`, `do_start_export`, the `blocking` wrapper pattern | `CountInInfo.{slow, count_in_bpm, silence_ms}`; `do_start_export` reads the track's flag; new `set_slow_count_in(id, on) -> ExportInfo` (2.4) |
| Frontend | `ex.info`, `loadInfo`, `countInText`, the shadcn `Switch` (as in `StemSettings.svelte`) | `setSlowCountIn(on)`; the switch row; new count-in texts; `export count-in` log line (2.5-2.6) |
| Player | everything | nothing. `metronome.rs`, `player*.rs` stay byte-identical |

### 2.2 Where the switch lives: `track.json` `exports.slow_count_in`

- **Format:** `"exports": {"slow_count_in": true, ...}`. Only the JSON value `true` means slow. A missing key, `false`
  or any other value means the normal count-in, so every existing track is "off" (spec requirement 4).
- **Writing:**
  - on: `set_slow_count_in(obj, true)` inserts the key (making `exports` an object first when it is missing or not an
    object, like `export_record::set`);
  - off: it removes the key, and removes `exports` when it ends up empty, so "off" looks exactly like "never set";
  - it returns whether anything changed. Unchanged means no write.
- **Why inside `exports`:**
  - the owner wants it in the track's own data, so a batch re-export keeps each track's choice;
  - `exports` is already the export-side part of `track.json`. `track_meta::revision` ignores it, so toggling the switch
    never causes a "changed on disk" conflict with an open Library edit (the same reason as the 2026-10-24 decision);
  - Library saves, group patches and the Editor's backing saves keep `exports` verbatim (`TrackMeta.extra`), and
    older builds keep it too. No schema bump.
- **Readers:** `export_record::read` is unchanged (it reads only `rc5` / `mp3` / `ogg`), so `TrackExports` and the
  "Last:" lines are unchanged. `clear_rc5` already keeps `exports` while it has other keys.
- **`TrackRecord.slow_count_in: bool`**, filled in `Repository::record` with
  `export_record::slow_count_in(m.extra.get(EXPORTS_KEY))`. It is serialized with the record (the frontend ignores it on
  `TrackRecord`; it reads `ExportInfo.count_in.slow`).
- **`Repository::set_slow_count_in(&self, id, on) -> Result<TrackRecord, String>`**, the same pattern as
  `record_export`:
  1. `raw_meta(id)`; problem tracks and tracks from a newer build are refused with the same errors;
  2. `export_record::set_slow_count_in`;
  3. `write_raw` only when it changed. `modified` and every other field stay as they are;
  4. log `calliope: export count-in id=<id> slow=<b> changed=<b>`;
  5. return `load_record(id)`.

  The caller holds the repository lock.

### 2.3 The count-in formula (spec requirements 1-3, 5)

`count_in_plan(map: &TempoMap, rate: u32, slow: bool) -> Option<CountInPlan>`, at the output rate R:

```
beat     = round(R * 60 / bpm)                  first section's BPM, unchanged
f        = 2 if slow else 1                     SLOW_SPACING = 2, a const in export_audio.rs
count_in = LEAD_BEATS * f * beat                4 or 8 beats of the track's tempo
silence  = count_in                             the lead silence (owner answer 2)
lead     = silence + count_in                   zeros written before the original (ticks mixed in)
first    = round(segments[0].start_ms * R / 1000)
skip     = first if the first beat is set else 0
tick[k]  = silence + (first - skip) + round(TICK_BEATS[k] * f * beat)
first_beat_frame = lead + first - skip
```

- **The fields:** `CountInPlan` keeps `bpm` (the track's), `beat_frames`, `lead_frames` (now silence + count-in),
  `first_beat_frame`, `skip_frames` and `ticks`. It gains `slow: bool`, `silence_frames` and `count_in_frames`.
- **`run` is unchanged.** It already writes `lead_frames` zeros, then the decoded original after dropping
  `skip_frames`, and mixes the ticks at their absolute frames. The progress estimate uses `lead_frames` and stays right.
- **Normal = today + silence.** With `slow = false` the new file is exactly `silence` zero frames followed by the old
  export. With the slow count-in, every tick is twice as far from the start of the count-in: 0, 2, 4, 5, 6, 7 beats.
  `TICK_BEATS[k] * 2` is a whole number, so there is no new rounding.
- **At 120 BPM, 44.1 kHz** (the acceptance criteria):
  - off: silence 0..88200 (2.0 s), ticks at 88200 + {0, 22050, 44100, 55125, 66150, 77175}, first beat at 176400
    (4.0 s);
  - on: silence 0..176400 (4.0 s), ticks at 176400 + {0, 44100, 88200, 110250, 132300, 154350}, first beat at 352800
    (8.0 s).
- **First beat not set, but the first section starts later** (e.g. fixture 406 at 0.5 s): as today, the ticks fall over
  the intro wherever it overlaps the count-in. The file is still `lead` zero frames + the whole original.
- **Requirement 3** ("the audio of the track itself is unchanged"): from `first_beat_frame` on, every sample is the
  original's from `first` on, bit for bit (no tick reaches the first beat: the last tick is half a beat, or one beat
  slow, before it).
- **The Player:** `LEAD_BEATS` and `TICK_BEATS` stay in `metronome.rs` with their values. The factor and the silence
  exist only in `export_audio.rs`.

### 2.4 When the switch is read, and the IPC

- **Read at start, like the tempo map.** `ExportState::start` already recomputes the count-in "from the map as it is
  now". `do_start_export` now also reads the flag of the planned track in the plan's repository, with
  `Repository::new(&plan.repo_root).load_record(&plan.id)?.slow_count_in`, and passes it to `start`. If the read fails,
  the start fails with that message. This way the file always matches what the pane shows when Yes is pressed in the
  overwrite question. `prepare_export` doesn't need the flag. The frontend also disables the switch while a prepare,
  a confirmation or an export is in progress (2.6), so the case shouldn't come up, but the order is defined anyway.
- **`CountInInfo`** (in `export_info`) becomes:

  ```rust
  pub struct CountInInfo {
      pub bpm: f64,           // the first section's BPM (unchanged)
      pub first_beat_ms: f64, // unchanged
      pub slow: bool,         // the track's switch
      pub count_in_bpm: f64,  // bpm, or bpm / 2 when slow
      pub lead_ms: f64,       // the count-in: LEAD_BEATS beats at count_in_bpm (= 4 * f beats at bpm)
      pub silence_ms: f64,    // the silence before it (= lead_ms)
      pub skip_ms: f64,       // unchanged
  }
  ```

  `lead_ms` keeps its meaning ("the count-in"), so it is still 2000 for 120 BPM normal. What the export adds in front
  of the first beat is `silence_ms + lead_ms`.
- **New command `set_slow_count_in(id: String, on: bool) -> ExportInfo`.** It is a `#[tauri::command] async` wrapper
  over `do_set_slow_count_in(repo, settings, store, id, on)` through `blocking`:
  1. under `repo.lock()`, `Repository::new(root.clone()).set_slow_count_in(&id, on)?`;
  2. release the lock, then return `do_export_info(repo, settings, store, &id)` (it takes the lock itself).

  It is registered in `build.rs` `COMMANDS`, the `gui.rs` `generate_handler!` and `capabilities/main.json`
  (`allow-set-slow-count-in`); `tests/frontend.rs` checks that the three agree. No refusal while an export runs: the
  running job's count-in was fixed at its start, and the frontend disables the switch then anyway.
- The `done` event's re-read `TrackRecord`s carry the flag. Nothing else about events changes.

### 2.5 Texts (spec requirement 6)

`countInText(info)` in `src/ui/lib/export-format.ts`, with `seconds(ms) = (ms / 1000).toFixed(1)`:

```
normal: Count-in: 4 beats at <bpm> BPM, after <silence> s of silence (adds <silence + lead> s<left>)
slow:   Count-in: 4 beats at <count_in_bpm> BPM (half of <bpm>), after <silence> s of silence (adds <silence + lead> s<left>)
<left> = "; the <skip> s before the first beat are left out" when skip_ms > 0, else ""
```

Examples:

| Case | Text |
|------|------|
| 180 BPM, slow | `Count-in: 4 beats at 90 BPM (half of 180), after 2.7 s of silence (adds 5.3 s)` (the spec's example) |
| 120 BPM, normal | `Count-in: 4 beats at 120 BPM, after 2.0 s of silence (adds 4.0 s)` |
| 120 BPM, normal, set first beat at 1.2 s | `Count-in: 4 beats at 120 BPM, after 2.0 s of silence (adds 4.0 s; the 1.2 s before the first beat are left out)` |

The BPMs are printed as plain JS numbers, as today (117.3 slow gives "58.65"). The old words "before the first beat"
are dropped from the main sentence: "after ... of silence" now says where the count-in sits.

### 2.6 The pane (spec requirements 1, 4, 7)

- **Markup.** In `ExportPane.svelte`, in the muted block, right after the `export-count-in` line:

  ```svelte
  <div class="flex min-h-6 items-center gap-3">
    <Switch id="export-slow-count-in" data-testid="export-slow-count-in"
      checked={info?.count_in?.slow ?? false} disabled={slowDisabled}
      onCheckedChange={(v) => void setSlowCountIn(v)} />
    <Label for="export-slow-count-in">Slow count-in (half tempo)</Label>
  </div>
  ```

  `Switch` comes from `$lib/components/ui/switch/index.js` and `Label` from `$lib/components/ui/label/index.js`, as
  in `StemSettings.svelte`.
- **State.** The switch has no local state. `checked` follows `ex.info`, so selecting another track shows that
  track's value as soon as its info has loaded.
- **When it is disabled.** `slowDisabled = !info?.count_in || !info.available || running || ex.preparing ||
  ex.confirm !== null || ex.slowSaving`. So it is disabled without a track or for a band or album row (`info` is null
  then), for a track that can't be exported, during a prepare (the folder dialog is open), during an overwrite
  question, during an export, and while its own write is running.
- **The row is always there** (disabled when there is nothing to switch), so it never makes the rows below jump.
- **`setSlowCountIn(on)`** in `export-state.svelte.ts`:
  1. return when `!ex.info`, `ex.slowSaving`, `ex.preparing`, `isRunning()` or `ex.confirm`;
  2. `const id = ex.info.id`, set `ex.slowSaving = true`, clear `ex.message`;
  3. `const info = await setSlowCountIn(id, on)` (ipc);
  4. `++infoSeq`, so an older `loadInfo` answer can't overwrite the new one. If `targetId === id`, set `ex.info = info`.
     The Memory box is never refilled here;
  5. log `export slow id=<id> on=<b> result=<saved|failed>`;
  6. on an error, `ex.message = String(e)` (the result line shows "Export failed: ..."; the switch stays as it was,
     because `ex.info` didn't change);
  7. finally `ex.slowSaving = false`.

  `resetExport()` clears `slowSaving`.
- **Count-in log.** A pane `$effect` logs `export count-in id=<id> slow=<b> text=<countInText(info)>` whenever the
  text or the id changes, the same pattern as `export target`. GUI tests read it.
- **Layout.**
  - The `Switch` is `h-6` (24 px). The muted block has `gap-1`, so the "Export..." group and every button move down
    by 28 px at 1280x800; task 9 measures the new positions on a screenshot.
  - The count-in line is not given more reserved height. With the fixtures (120 BPM, no skip) the longest slow text
    is about 80 characters: one line at 1280x800. A long text (skip, odd BPMs) wraps and pushes the rows below
    down, as it already did at 1024x640 (docs/ui.md, gui-export).
  - The earlier estimate (rc5-quick) left about 60 px to spare at 1280x800, so `overflow <= 14` should still hold.
    If it doesn't, the implementer reports it instead of shrinking other rows.
  - "Fits" means the same as in rc5-quick: at 1280x800, `overflow <= 14` and `hscroll <= 0`; at 1024x640,
    `hscroll <= 0`, and vertical scrolling is allowed as before. Screenshots of both are reviewed.

### 2.7 Logs

- **Rust (prefix `calliope: `):**
  - `export count-in id=<id> slow=<b> changed=<b>`;
  - `export start job=<j> id=<id> target=<t> path=<p> rate=<hz> channels=<c> count_in=<bpm> slow=<b>
    first_beat_ms=<ms> silence_frames=<n> lead_frames=<n> skip_frames=<n> overwrite=<b>`, with `lead_frames` now
    silence + count-in.
- **Frontend (prefix `calliope-ui: `):**
  - `export slow id=<id> on=<b> result=<saved|failed>`;
  - `export count-in id=<id> slow=<b> text=<text>`.

## 3. Tasks

The coordinator runs the full `npm test` (in `src/calliope-gui`) once per task. Implementers run only the tests named in
the task. All Rust is formatted with `cargo fmt --all`. No task may change `src/calliope-gui/src/metronome.rs` or any
`src/calliope-gui/src/player*.rs`.

### Task 1: Storage: `exports.slow_count_in`, `TrackRecord.slow_count_in`, `Repository::set_slow_count_in`
- files: `src/calliope-gui/src/export_record.rs`, `src/calliope-gui/src/repository.rs`,
  `src/calliope-gui/src/repository_tests.rs`, `src/calliope-gui/src/export_rules.rs` (only the `TrackRecord` literal in
  its tests: `slow_count_in: false`)
- does:
  - `export_record.rs`:
    - `pub const SLOW_COUNT_IN_KEY: &str = "slow_count_in";`
    - `pub fn slow_count_in(exports: Option<&Value>) -> bool`: true only for an object holding exactly the value `true`
      under the key;
    - `pub fn set_slow_count_in(obj: &mut Map<String, Value>, on: bool) -> bool`, as in 2.2;
    - unit tests: reading `true`, `false`, `"yes"`, `1`, `null`, a missing key, a non-object `exports` and `None`;
      setting on into an empty object, into a non-object `exports` and next to `rc5` / `mp3` / unknown keys (kept);
      off removes the key and an emptied `exports`, keeps a non-empty one, and returns false when nothing changes;
      `read()` of an object with the key gives the same `TrackExports` as without it;
  - `repository.rs`: the `TrackRecord.slow_count_in` field (doc comment: "the export's slow count-in, from
    `exports.slow_count_in`"), filled in `record`, and `set_slow_count_in` (2.2), with its log line. Fix the
    `TrackRecord` literal in `repository_tests.rs` (around line 1567);
  - `repository_tests.rs`, new tests:
    - a fixture track reads `slow_count_in == false`;
    - on: the record says true, `track.json` has `exports.slow_count_in == true`, `modified` and `revision` are
      unchanged, and every other key is equal as JSON;
    - on again: no write (the file's mtime/bytes are unchanged);
    - off: the key is gone, and so is `exports` when nothing else was in it; with an `rc5` entry, `exports` stays;
    - a Library save of the track (`save` with the old revision) succeeds and keeps the key;
    - a problem track (bad `track.json`) is refused and not written.
- done when: `cargo test -p calliope-gui --lib export_record`, `cargo test -p calliope-gui --lib repository` and
  `cargo test -p calliope-gui --lib export_rules` pass; `cargo clippy -p calliope-gui --all-targets -- -D warnings` is
  clean.
- test files touched: `src/export_record.rs` (tests module), `src/repository_tests.rs`, `src/export_rules.rs` (literal).
- routine: no

### Task 2: Lead silence and the slow count-in in the audio and the job; existing tests follow
- files: `src/calliope-gui/src/export_audio.rs`, `src/calliope-gui/src/export_audio_tests.rs`,
  `src/calliope-gui/src/export_job.rs`, `src/calliope-gui/src/export_job_tests.rs`,
  `src/calliope-gui/src/export_ipc.rs` (only `do_start_export`), `src/calliope-gui/tests/acceptance_export.rs`
  (existing tests only)
- does:
  - `export_audio.rs`:
    - `SLOW_SPACING`, `count_in_plan(map, rate, slow)` and the new `CountInPlan` fields, exactly as in 2.3;
    - update the module and function doc comments (4 beats of silence, then the count-in);
  - `export_job.rs`:
    - `ExportState::start` gains `slow: bool` after `tempo` and calls `count_in_plan(tempo, pl.rate, slow)`;
    - the `export start` log line as in 2.7;
  - `export_ipc.rs` `do_start_export`: read the flag (2.4) and pass it;
  - `export_audio_tests.rs`: update the plan tests to the new frames, and add these:
    - normal at 120 BPM / 44.1 kHz: `silence_frames == count_in_frames == 88200`, `lead_frames == 176400`, the ticks
      as in 2.3, `first_beat_frame == 176400`;
    - slow: `silence_frames == count_in_frames == 176400`, ticks `176400 + {0, 44100, 88200, 110250, 132300, 154350}`,
      `first_beat_frame == 352800`;
    - slow with a set first beat and with an unset later first section (`skip` and `first` as today);
    - for any BPM, `plan(slow).ticks[k] - plan(slow).silence_frames == 2 * (plan(normal).ticks[k] -
      plan(normal).silence_frames)` when `first - skip == 0`;
    - `metronome::LEAD_BEATS == 4` and `TICK_BEATS` are unchanged (a guard for the Player);
  - `export_job_tests.rs`: the rig's `start` helper passes `slow: false`, and the frame expectation near line 632
    follows the new lead. A new test runs an RC-5 job with `slow: true` (a pure-Rust temp drive, as the rig does) and
    checks the WAV length = `352800 + original frames` and silence before the first tick;
  - `tests/acceptance_export.rs`, **existing tests only.** Every exported file now has `silence` more frames in front:
    - `assert_lead_then_audio` takes the silence into account: `lead = 8 * beat`, ticks at `4 * beat + ticks_for(0, beat)`,
      zeros before the first tick;
    - `ac_count_in_before_the_first_beat` (lead values 176400 / 211680, ticks + silence);
    - `ac_rc5_memory_7` (`l[..88200]` is pure silence now, so the tick check moves to `88200..176400`);
    - the `tick_onset` callers (`lead` = silence + count-in, ticks + silence);
    - `ac_long_intro_is_kept...`;
    - the `ac_first_beat_*` constants 88200 → 176400;
    - `ac_first_beat_track_saved_before_the_feature_exports_everything`.

    The meaning of each test stays the same. Only frame numbers and comments change. `ac_count_in_ignores_speed_and_bar_length`
    (`lead_ms == 2000`) stays as it is.
- done when: `cargo test -p calliope-gui --lib export_` and `cargo test -p calliope-gui --test acceptance_export` pass;
  `cargo test -p calliope-gui --lib player` and `cargo test -p calliope-gui --lib metronome` pass, unchanged;
  `git diff --stat` shows no change to `metronome.rs` or `player*.rs`; clippy is clean.
- test files touched: `src/export_audio_tests.rs`, `src/export_job_tests.rs`, `tests/acceptance_export.rs`.
- routine: no

### Task 3: Export IPC: `CountInInfo` fields and `set_slow_count_in`
- files: `src/calliope-gui/src/export_ipc.rs`, `src/calliope-gui/src/export_ipc_tests.rs`
- does:
  - `CountInInfo` as in 2.4, filled in `do_export_info` from `rec.slow_count_in`;
  - `do_set_slow_count_in` and the `#[tauri::command] set_slow_count_in` wrapper (not registered yet, task 4);
  - `export_ipc_tests.rs`:
    - update the `CountInInfo` literals (`slow: false`, `count_in_bpm` = bpm, `silence_ms` = `lead_ms`);
    - `do_set_slow_count_in(…, true)` returns info with `slow: true`, `count_in_bpm: 60.0`, `lead_ms: 4000.0`,
      `silence_ms: 4000.0` for track 401, and a second `do_export_info` (a fresh `RepoState` on the same folder, i.e. a
      restart) still says true; off goes back to the normal values;
    - an unknown id is an error and writes nothing;
    - export after `set_slow_count_in(true)`: `do_prepare_export` + `do_start_export` produce an RC-5 WAV whose length
      is `352800 + original` (the flag is read at start);
    - prepare with the switch off, then `set_slow_count_in(true)`, then start: the file is slow (2.4).
- done when: `cargo test -p calliope-gui --lib export_ipc` passes; clippy is clean.
- test files touched: `src/export_ipc_tests.rs`.
- routine: no

### Task 4: Register the `set_slow_count_in` command
- files: `src/calliope-gui/build.rs`, `src/calliope-gui/src/gui.rs`, `src/calliope-gui/capabilities/main.json`
- does: three one-line insertions, nothing else:
  - `build.rs`: in `const COMMANDS`, after the line `    "choose_rc5_folder",` add the line
    `    "set_slow_count_in",`;
  - `src/gui.rs`: in `generate_handler![...]`, after the line `            export_ipc::choose_rc5_folder,` add the line
    `            export_ipc::set_slow_count_in,`;
  - `capabilities/main.json`: after the line `    "allow-choose-rc5-folder",` add the line
    `    "allow-set-slow-count-in",`.
- done when: `cargo test -p calliope-gui --test frontend` passes (it checks that the three lists agree) and
  `cargo build -p calliope-gui` succeeds.
- test files touched: none (the existing `tests/frontend.rs` checks it).
- routine: yes

### Task 5: Headless acceptance tests
- files: `src/calliope-gui/tests/acceptance_export.rs`
- does: new `ac_slow_*` tests on the existing rig (fixture copies in temp dirs; the RC-5 is a temp folder with `ROLAND/`;
  the real local `ffmpeg`/`ffprobe`). Add a rig helper `slow(n, on) -> ExportInfo` calling `do_set_slow_count_in`. The
  tests:
  - `ac_slow_off_120_bpm_silence_ticks_first_beat`: track 401 (120 BPM, start 0), RC-5 → `wl[..88200]` all exactly 0.0;
    tick onsets (`tick_onset`) at `88200 + {0, 22050, 44100, 55125, 66150, 77175}` (± 3 frames); `wl[176400 + k] ==
    ol[k]` for every k (left and right); length `176400 + ol.len()`;
  - `ac_slow_on_120_bpm_silence_ticks_first_beat`: the same with `slow(401, true)`: zeros up to 176400; onsets at
    `176400 + {0, 44100, 88200, 110250, 132300, 154350}`; zeros between the ticks; `wl[352800 + k] == ol[k]`;
  - `ac_slow_audio_from_the_first_beat_is_unchanged`: for both settings and for tracks 401, 406 (a first section at
    0.5 s, not set) and 411 (a set first beat, the `add_noisy` fixture), every sample from `first_beat_frame` on equals
    the original from `first` on;
  - `ac_slow_first_beat_set_still_drops_the_audio_before_it`: 411 with `found_ms` at 1.2 s, slow on → length
    `16 * 22050 + NOISY_FRAMES - 52920`, and none of the noise of the first 1.2 s is in the file;
  - `ac_slow_per_track_survives_a_restart`: `slow(A, true)`, `slow(B, false)`. A new `RepoState` / settings on the same
    temp folder (= restart) → `do_export_info(A).count_in.slow == true`, B false, a never-set track false. A's
    `track.json` has `exports.slow_count_in: true`, B's none. `repo_unchanged_except_exports()` holds, and A's revision is
    unchanged;
  - `ac_slow_mp3_and_ogg_have_the_same_count_in_and_silence`: slow on, MP3 and OGG → decoded length `= 8.0 s + original`
    (± 0.06 s); the first 4.0 s quiet (peak below 0.01); a 1500 Hz Goertzel tick at `4.0 + {0, 1, 2, 2.5, 3, 3.5}` s; the
    440 Hz tone of the original after 8.0 s;
  - `ac_slow_info_texts_values`: a 180 BPM tempo written with `write_tempo` → normal `lead_ms`/`silence_ms` 1333.3,
    slow `count_in_bpm` 90 and 2666.7 (± 0.1);
  - `ac_slow_toggle_writes_only_the_key_and_survives_a_library_save`: the toggle leaves every other file byte-identical
    and `track.json` equal as JSON apart from `exports`. A Library save with the old revision succeeds and keeps the
    key.
- done when: `cargo test -p calliope-gui --test acceptance_export` passes.
- test files touched: `tests/acceptance_export.rs`.
- routine: no

### Task 6: Frontend types, state and texts
- files: `src/calliope-gui/src/ui/lib/ipc.ts`, `src/calliope-gui/src/ui/lib/export-state.svelte.ts`,
  `src/calliope-gui/src/ui/lib/export-format.ts`, `src/calliope-gui/src/ui/lib/export-format.test.ts`,
  `src/calliope-gui/src/ui/lib/ipc-export.test.ts`, `src/calliope-gui/src/ui/views/ExportView.test.ts` (only the
  `CountInInfo` literal near line 49 and the count-in text expectation near line 103)
- does:
  - `ipc.ts`: `CountInInfo` gains `slow: boolean; count_in_bpm: number; silence_ms: number`. Add the wrapper
    `setSlowCountIn = (id: string, on: boolean): Promise<ExportInfo> => invoke<ExportInfo>('set_slow_count_in', { id, on })`;
  - `export-state.svelte.ts`: `ex.slowSaving`, and `setSlowCountIn(on)` as in 2.6 (imported from ipc under another
    name, e.g. `saveSlowCountIn`), cleared in `resetExport()`;
  - `export-format.ts`: `countInText` as in 2.5;
  - `export-format.test.ts`: the `info()` literal gets the new fields. Assert the three texts in the table of 2.5
    (180 slow, 120 normal, 120 normal with skip), a slow one with a skip, and `''` for `null` and `count_in: null`;
  - `ipc-export.test.ts`: `setSlowCountIn('t1', true)` invokes `set_slow_count_in` with `{ id: 't1', on: true }`;
  - `ExportView.test.ts`: the literal gets `slow: false, count_in_bpm: 120, silence_ms: 2000`. The expected line becomes
    `Count-in: 4 beats at 120 BPM, after 2.0 s of silence (adds 4.0 s)`.
- done when: in `src/calliope-gui`, `npx vitest run src/ui/lib/export-format.test.ts src/ui/lib/ipc-export.test.ts
  src/ui/views/ExportView.test.ts` passes and `npm run check` is clean.
- test files touched: `src/ui/lib/export-format.test.ts`, `src/ui/lib/ipc-export.test.ts`, `src/ui/views/ExportView.test.ts`.
- routine: no

### Task 7: The switch in the Export pane
- files: `src/calliope-gui/src/ui/components/export/ExportPane.svelte`, `src/calliope-gui/src/ui/views/ExportView.test.ts`
- does:
  - the switch row, `slowDisabled` and the `export count-in` log effect (2.6);
  - `ExportView.test.ts`: the mock answers `set_slow_count_in` (it returns the info with `slow` flipped and the slow
    values) and records the calls. New tests:
    1. the switch (`role="switch"`, name "Slow count-in (half tempo)") is unchecked for a normal track;
    2. a click sends `{ id, on: true }`, then the switch is checked and the count-in line reads
       `Count-in: 4 beats at 60 BPM (half of 120), after 4.0 s of silence (adds 8.0 s)`; a second click sends
       `on: false` and the normal text comes back;
    3. a track whose info says `slow: true` shows it checked when selected; selecting a normal one unchecks it;
    4. disabled with no track, for a group row, for an unavailable track, while preparing (a pending `prepare_export`),
       while the overwrite question is open, and while an export runs;
    5. an error from `set_slow_count_in` shows `Export failed: <e>` and leaves the switch and the text unchanged;
    6. the Memory box text is not refilled by a toggle (type `12`, toggle, it still reads `12`).
- done when: `npx vitest run src/ui/views/ExportView.test.ts` passes; `npm run check` is clean.
- test files touched: `src/ui/views/ExportView.test.ts`.
- routine: no

### Task 8: README
- files: `src/calliope-gui/README.md`
- does: in the "## Export" section, replace the bullet that starts with `* Count-in: every export starts with 4 beats of
  lead` (three lines, ending with `The exported file is therefore 4 beats longer than the original.`) with exactly
  this text (ASCII only):

  ```
  * Count-in: every export starts with silence, then a count-in of six ticks at the track's
    tempo (the last one half a beat before the track's first beat), then the track. The silence
    is as long as the count-in (4 beats), so the file is 8 beats longer than the track from its
    first beat on. That leaves time to get ready after pressing play on the RC-5.
  * "Slow count-in (half tempo)": for fast tracks. With this switch on, the six ticks are twice
    as far apart (the count-in is 4 beats at half the tempo, 8 beats of the track) and the
    silence before them is twice as long too. The switch is remembered per track (in the
    track's `track.json`, under `exports`), so re-exporting a track keeps its choice. The line
    above the switch says which count-in will be written and how much it adds.
  ```
- done when: `grep -n 'Slow count-in (half tempo)' src/calliope-gui/README.md` finds the line,
  `grep -c 'The exported file is therefore 4 beats longer' src/calliope-gui/README.md` prints 0,
  `LC_ALL=C grep -nP '[^\x00-\x7F]' src/calliope-gui/README.md` prints nothing, and `git diff --stat` shows only
  README.md changed.
- test files touched: none.
- routine: yes

### Task 9: GUI end-to-end tests, visual review, docs/ui.md
- files: `src/calliope-gui/tests/gui_export_e2e.rs`, `docs/ui.md`
- does:
  - **Coordinates:** `BTN_RC5`, `BTN_MP3`, `BTN_OGG` and `MEMORY_BOX` move down by the switch row (expected +28 px;
    take them from a 1280x800 screenshot). Add `SLOW_SWITCH` (the switch itself, also from the screenshot).
  - **Existing assertions that follow the lead silence:**
    - `export_rc5_flow`: `l.len() == 44100 * 4 + 176400`, and the other length check near line 406;
    - `export_mp3_and_ogg`: duration `8.0` ± 0.2 ("4 s + 2 s silence + 2 s count-in").
  - **New test `export_slow_count_in`** (RC-5 drive from `rc5()`, answers `export-rc5 <B>` twice):
    1. Select EX_PLAIN → the `export count-in id=<EX_PLAIN> slow=false text=Count-in: 4 beats at 120 BPM, after 2.0 s of
       silence (adds 4.0 s)` line appears.
    2. Click `SLOW_SWITCH` → `export slow id=<EX_PLAIN> on=true result=saved`, then `export count-in ... slow=true
       text=Count-in: 4 beats at 60 BPM (half of 120), after 4.0 s of silence (adds 8.0 s)`. `track.json` has
       `exports.slow_count_in == true`.
    3. Shots `export-slow-1280`, then at 1024x640 `export-slow-1024`, then back to 1280x800. Check `overflow <= 14` and
       `hscroll <= 0` at 1280, and `hscroll <= 0` at 1024.
    4. Memory 5, RC-5 → done; `wav(B, 5)` is `352800 + original` frames long, zeros before frame 176400, and a tick
       near 176400. The `export start` line says `slow=true silence_frames=176400 lead_frames=352800`.
    5. Select EX_MONO → `export count-in id=<EX_MONO> slow=false`. Select EX_PLAIN again → `slow=true`.
    6. Close the app, start it again on the same dirs (the second run, as in `export_rc5_flow`): EX_PLAIN → `slow=true`,
       EX_MONO → `slow=false`.
    7. Click `SLOW_SWITCH` on EX_PLAIN → off, `track.json` has no `slow_count_in`. RC-5 memory 5 → the overwrite
       question (`Return` on Yes as in `export_rc5_flow`) → done, and the file is `176400 + original` long.
    8. At the end: every answer is used, there is no `script mismatch` line, the repository is unchanged apart from
       `exports` (`snapshot_except_exports`), and `assert_no_real_audio` and `no_csp_violation` pass.
  - **Review the screenshots:**
    - the switch and its label sit under the count-in line without overlapping it;
    - the slow text fits on one line at 1280x800 and wraps cleanly at 1024x640;
    - the buttons sit where the constants say.
  - **`docs/ui.md`:** add a `(gui-export-slow-count-in)` bullet after the gui-export-rc5-quick one. It covers the switch
    row, when it is disabled, the new count-in texts, the 28 px shift, and the measured 1280x800 / 1024x640 values.
- done when: in `src/calliope-gui`, `npm run build`, then `npm run test:gui -- -- export_` passes for `gui_export_e2e`
  (the coordinator runs the full `npm run test:gui`), and the screenshots in `target/gui-shots/` were reviewed.
- test files touched: `tests/gui_export_e2e.rs`.
- routine: no

## 4. Test strategy

- **Unit (Rust):**
  - `export_record` key read/write, leniency, and the empty-`exports` cleanup (task 1);
  - `Repository::set_slow_count_in`: no `modified` or revision change, no write when unchanged, survives a Library
    save, problem tracks refused (task 1);
  - `count_in_plan` frames for normal / slow, a set or unset first beat, odd BPMs, and the Player constants unchanged
    (task 2);
  - the job writes a slow file (task 2);
  - `CountInInfo` values, `do_set_slow_count_in`, and the flag read at start (task 3).
- **Acceptance (Rust, headless):** `tests/acceptance_export.rs`:
  - every existing export test, updated for the lead silence (task 2);
  - the new `ac_slow_*` tests (task 5).

  Everything runs on fixture copies in temp dirs, with the real local ffmpeg/ffprobe; the RC-5 is a temp folder with
  `ROLAND/`. Audio is checked exactly for WAV (zeros, tick onsets, bit-exact audio from the first beat) and by
  duration/Goertzel for MP3/OGG. "Restart" = new state objects on the same temp repository.
- **Frontend (vitest + jsdom):** `export-format.test.ts` (texts), `ipc-export.test.ts` (the command and its arguments),
  `ExportView.test.ts` (switch state per track, toggling, disabled states, errors, the Memory box untouched).
- **GUI e2e:** `tests/gui_export_e2e.rs` `export_slow_count_in`: the real app on a private display, scripted folder
  dialogs, capture audio. It covers a real switch click, the log lines, `track.json`, the WAV frames, a real restart,
  and the layout lines plus screenshots at 1280x800 and 1024x640. `export_rc5_flow` and `export_mp3_and_ogg` keep their
  checks with the new lengths.
- **Never touched:** a real RC-5, real drives, the user's repository or settings, the real system dialogs, any audio
  device, the calliope-stems server or any other host. No symphonia.
- **Whole suite, in `src/calliope-gui`:**
  - `npm test`: `cargo fmt --all --check`, svelte-check, vitest, vite build, `cargo test --workspace`, clippy
    `-D warnings`;
  - `npm run test:gui`: all GUI binaries, each on its own private display.

## 5. Deployment

Laptop app only: `npm run build:app` in `src/calliope-gui` and install as before. There is one new Tauri command
(`set_slow_count_in`, in the app's own ACL). `track.json` gains the optional `exports.slow_count_in`; older builds keep
it (it is inside `exports`) and ignore it. Nothing changes on archserver or in calliope-stems.

## 6. Manual checks (owner, laptop with the real RC-5)

1. **Normal count-in with silence:** pick a fast track (e.g. 180 BPM) with the switch off. The line reads "Count-in: 4
   beats at 180 BPM, after 1.3 s of silence (adds 2.7 s)". Export it to an RC-5 memory, eject, and play that memory on
   the RC-5. You hear about 1.3 s of silence, then "one, two, one-two-three-four" at 180, and the track comes in on the
   beat after.
2. **Slow count-in:** turn "Slow count-in (half tempo)" on. The line at once reads "Count-in: 4 beats at 90 BPM (half
   of 180), after 2.7 s of silence (adds 5.3 s)". Export to the same memory (answer Yes to the replace question) and
   play it. You get 2 bars of silence, then the count at half speed, then the track at full speed on the beat after the
   last tick. Check that you can come in on time.
3. **Per track:** leave the switch on for track A and off for track B. Export both. Close and restart Calliope:
   selecting A shows the switch on, B off. Re-export A and check that it still has the slow count-in.
4. **First beat:** on a track with a set first beat (Find start in the Player), check that the export still starts
   right at the first beat after the count-in, with nothing of the intro.
5. **MP3 / OGG:** with the switch on, export an MP3 and an OGG and play them in any player. They have the same silence
   and slow count-in.
6. **RC-5 loop playback:** with the RC-5 set to loop, check how the silence and the count-in sound at each repeat.
   This was already true for the count-in before; tell us if the silence makes the loop restart awkward.
7. **Look:** at your usual window size and at a small one (about 1024x640), the switch sits under the count-in line,
   nothing overlaps, and the buttons below are usable.

## 7. Acceptance mapping

| Criterion / requirement | Tasks | Tests |
|---|---|---|
| 120 BPM, switch off → 2.0 s silence, ticks at 0, 1, 2, 2.5, 3, 3.5 beats after it, first beat at 4.0 s (req. 2, 3) | 2, 5, 9 | `count_in_plan` normal test; `ac_slow_off_120_bpm_silence_ticks_first_beat`; updated `ac_count_in_*`; GUI `export_slow_count_in` step 7, `export_rc5_flow` length |
| Switch on → 4.0 s silence, ticks at 0, 2, 4, 5, 6, 7 beats after it, first beat at 8.0 s (req. 1, 3) | 2, 3, 5, 9 | `count_in_plan` slow test; `export_job_tests` slow job; `export_ipc_tests` slow export; `ac_slow_on_120_bpm_silence_ticks_first_beat`; GUI `export_slow_count_in` step 4 |
| Every sample from the first beat on identical (shifted) (req. 3) | 2, 5 | `ac_slow_audio_from_the_first_beat_is_unchanged` (401, 406, 411, both settings); updated `assert_lead_then_audio` |
| A set first beat still drops the audio before it | 2, 5 | `ac_slow_first_beat_set_still_drops_the_audio_before_it`; updated `ac_first_beat_*` |
| On for A, off for B, restart → A on, B off (req. 4) | 1, 3, 5, 7, 9 | repository tests; `export_ipc_tests` fresh `RepoState`; `ac_slow_per_track_survives_a_restart`; `ExportView.test.ts` test 3; GUI `export_slow_count_in` steps 5-6 |
| A track never set uses the normal count-in (req. 4) | 1, 5 | `export_record` read tests; `ac_slow_per_track_survives_a_restart` (never-set track) |
| Switch on → MP3 and OGG have the same slow count-in and silence (req. 5) | 2, 5, 9 | `ac_slow_mp3_and_ogg_have_the_same_count_in_and_silence`; GUI `export_mp3_and_ogg` (8.0 s, normal) |
| Switch toggled → the count-in line changes at once (req. 6) | 3, 6, 7, 9 | `export-format.test.ts`; `ExportView.test.ts` test 2; `ac_slow_info_texts_values`; GUI step 2 (`export count-in ... text=`) |
| Export view at 1280x800 and 1024x640 → nothing overflows (req. 7) | 7, 9 | GUI `export_slow_count_in` step 3 (`overflow <= 14`, `hscroll <= 0`) + screenshots `export-slow-1280` / `-1024`; `export_view_buttons`, `export_rc5_quick` keep their layout checks |
| The Player's count-in is unchanged (context, out of scope) | 2 | `metronome::LEAD_BEATS` / `TICK_BEATS` guard test; `player` / `metronome` lib tests unchanged; `git diff --stat` |

## 8. Open questions

None. These points were decided here, and the owner can overrule any of them:
- **Storage:** the switch is `exports.slow_count_in` in `track.json`. It doesn't change the track's revision, so an open
  Library edit never conflicts with it.
- **When it is read:** at the export's start (like the tempo map). The switch is disabled while a folder dialog, an
  overwrite question or an export is open.
- **"Silence as long as its count-in":** with a first section that starts later and no set first beat, the file is
  still `silence + count-in` zero frames + the whole original, so the ticks overlap the intro as they do today.
- **The count-in line:** "before the first beat" gives way to "after <s> s of silence". "adds" is silence + count-in.
