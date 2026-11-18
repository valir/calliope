# Plan: gui-import-normalize (normalize the full mix as the first step of every import)

Spec: `specs/gui-import-normalize.md`. The spec's open questions were settled by the owner on 2026-11-18: the
target is the backings' (-14 LUFS integrated, true peak at most -1 dBTP, no limiter); the step is always on;
the kept `original.flac` is the normalized mix; the result is shown and logged, not stored in `track.json`.

## 1. Summary

Every import (URL, local audio, local video, each file of a directory) gets a new step between the ffmpeg
conversion and the `ready` state: the converted `audio.flac` is measured with the existing meter and, unless it
is already at the target or too quiet/short, a scaled copy `normalized.flac` is written next to it. That copy is
what is uploaded for stem extraction and what becomes `original.flac`. The progress shows a "Normalizing" step,
the edit pane shows the result line, the directory summary shows it per file, and the log records the measured
loudness, true peak and gain. A small `normalize.json` marker makes a resumed URL import reuse the result
instead of normalizing again.

## 2. Design

### 2.1 What already exists and is reused

- `src/loudness.rs`: `Meter`, `Measure`, `decide(m, Purpose::Normalize)` (gain = min(-14 - L, -1 - TP); within
  0.1 LU/dB of the target or the ceiling = unchanged (`at-target` / `at-ceiling`); no gated loudness (silence,
  under 0.4 s, below -70 LUFS) = `too-quiet`), `db_to_gain`, `LoudnessResult` + `result_of`. Nothing in it
  changes. `Purpose::Normalize` is the right purpose: it is the only one with the 0.1 LU "not changed" rule that
  requirement 5 asks for.
- `src/backing_render.rs`: `RenderInput`/`RenderStem`, `measure_mix` (pass 1, unclipped, cancellable,
  progress per 4096 frames) and `render_flac` (pass 2: gain, clip, quantise, `flacenc`, `create_new` part file,
  fsync, part removed on error and on cancel, STREAMINFO MD5 returned). A one-"stem" render with a gain is
  exactly the backing Normalize's scaled copy (`backing_normalize.rs` step 6). `fmt_opt` formats log values.
- `src/stem_audio.rs`: `StemReader` (channels, bits, frames, STREAMINFO MD5).
- `src/import_job.rs` / `src/import_batch.rs`: the preparation thread (`run_url`, `prepare_file`, `run_one`),
  `Ctx::phase` / `Ctx::progress` (throttled to 10/s), `Stop::Cancelled` / `Stop::Failed`, `finish_prepare`
  (cancel or failure before `ready` ends the job; a local-file job's folder is removed, a URL job's folder
  stays for resume), `media::Cancel` (an `AtomicBool` via `flag()`).
- No new crate. No `symphonia`. Claxon decodes, `flacenc` encodes, `ebur128` measures, as today.

### 2.2 Where the step sits (requirements 1, 2, 3)

```text
url:   download (or resume) -> preparing (probe, FLAC) -> normalizing -> ready(metadata, loudness)
file:  preparing (probe, FLAC)                         -> normalizing -> ready(metadata, loudness)
batch: per file: preparing -> normalizing -> uploading -> ... -> saved | failed (no edit pane)
ready --Extract--> uploading (the normalized mix) -> queued -> working -> receiving -> saving
                   (keep original: the normalized mix is moved in as original.flac)
```

- The step runs on the preparation thread, after `media::to_flac` and before `set_ready`, in two places only:
  `run_url` (after the `to_flac` call or the "audio.flac already there" skip) and `prepare_file` (used by local
  audio, local video and every batch file). So all four sources get it with no per-source code.
- Its output path replaces `audio` everywhere after it: `Job.audio` (what `start_extraction` uploads and
  `adopt_original` moves into the track) and `Prepared.audio` (what the batch hands to `extract_cleaned`).
  Nothing downstream changes: the stems are separated from the normalized mix and the kept original is the
  normalized mix (requirement 3, owner answer 3).
- Why before `ready` and not at Extract: the result must be shown with the track before saving
  (requirement 4) and the batch has no edit pane; doing it in the preparation also gives it the preparation's
  cancel/failure behaviour for free.

### 2.3 The step: `src/import_normalize.rs` (new, pure, no Tauri)

```rust
pub const NORMALIZED_FLAC: &str = "normalized.flac";
pub const MARKER_FILE: &str = "normalize.json";
pub const PART_FILE: &str = ".normalized.flac.part";
/// The normalized copy is always written as 24-bit FLAC.
pub const OUTPUT_BITS: u32 = 24;

#[derive(Debug, Clone, PartialEq)]
pub struct Normalized {
    /// The mix the import goes on with: `<dir>/normalized.flac`, or `converted` itself when unchanged.
    pub audio: PathBuf,
    /// What the meter said about the converted file.
    pub measured: Measure,
    pub decision: Decision,
    /// `loudness::result_of(measured, decision)`: the values of `audio`.
    pub result: LoudnessResult,
    /// A new file was written (outcome normalized or limited).
    pub changed: bool,
    /// Taken from a valid `normalize.json`: nothing was measured or written.
    pub resumed: bool,
}

/// `converted` is `<dir>/audio.flac`. `progress` gets 0.0..0.4 while measuring and 0.4..1.0 while writing
/// (the backing Normalize's split). A set `cancel` returns `Err(backing_render::CANCELLED)`.
pub fn normalize(dir: &Path, converted: &Path, cancel: &AtomicBool,
                 progress: &mut dyn FnMut(f32)) -> Result<Normalized, String>;

/// The result recorded by an earlier run in `dir`, if `normalize.json` is valid for `converted` (2.5).
pub fn resume(dir: &Path, converted: &Path) -> Option<Normalized>;
```

`normalize` does, in order:

1. `resume(dir, converted)`: a valid marker is returned at once (`resumed: true`, `progress` never called).
2. Removes a stale `normalize.json` and `normalized.flac` of this folder if present (regular files only, via
   `symlink_metadata`; the folder is the job's own, as for `media::to_flac`'s part file).
3. Opens `converted` with `StemReader`: 2 channels, 16 or 24 bits, 44.1 kHz from the conversion, `frames()`
   present and > 0; anything else is an error ("the converted audio has an unsupported format" / "has no
   length in its header"). Keeps its STREAMINFO MD5 (`source_md5`).
4. `measure_mix` over `RenderInput { stems: [converted, gain 1.0], sample_rate, bits: OUTPUT_BITS, frames }`
   (`bits: 24` so `open_readers` accepts 16- and 24-bit input), progress `f * 0.4`.
   `d = loudness::decide(m, Purpose::Normalize)`.
5. Unchanged (`at-target`, `at-ceiling`, `too-quiet`): writes the marker with `"file": "audio.flac"` and
   returns `audio = converted`. The mix sent is then the converted file itself, byte for byte (AC 3, AC 4).
6. Changed (`normalized`, `limited`): removes a leftover `PART_FILE` (a regular file; anything else at that
   name is an error, "cannot clear .normalized.flac.part"), then `render_flac` with
   `gain = db_to_gain(d.gain_db)` and `bits: OUTPUT_BITS` into the part file, progress `0.4 + f * 0.6`;
   `rename` part -> `normalized.flac`; then writes the marker (`fsutil::write_atomic`). Returns
   `audio = dir/normalized.flac`.

Peaks and full scale (requirement 5): the gain never lifts the true peak above -1 dBTP (about 0.891 of full
scale), and a sample peak is never above the true peak, so no rendered sample reaches full scale; `render_flac`
still clips at +-1 and counts clipped samples, which is logged (`clipped=<n>`, always 0 in practice) as a
guard. A negative gain (loud masters, or `limited` with a peak already above -1 dBTP) only lowers peaks.

Bit depth (team decision, 2.7): the copy is always 24-bit. ffmpeg already gives 24-bit FLAC for every lossy or
float source (checked with ffmpeg 9.0 on the fixtures: MP3, Ogg, AAC and float WAV give `s32`/24); only 16-bit
lossless sources (CD rips) come in as 16-bit. Writing 24 bits adds no new rounding on top of the source's own
quantisation when the gain is up (a +10 dB gain on a 16-bit file would otherwise re-round every sample) and
costs size only for those 16-bit sources (worst case, 15 minutes of 24-bit stereo is about 240 MB raw, below
the server's 300 MB upload limit). Everything downstream already reads 16 and 24 bits (claxon, the server's
check, the separator, the Player, the Editor).

Errors are plain text; `import_job` maps them (2.6).

### 2.4 Files in the job folder

```text
import-tmp/url-<key>/ | file-<uuid>/
  job.json
  download.<ext>          (URL only)
  info.json               (URL only)
  audio.flac              the converted file; never modified after the conversion
  .normalized.flac.part   while writing (removed on error and cancel)
  normalized.flac         the normalized mix (only when the gain is not 0)
  normalize.json          the marker (written last)
```

`normalize.json` (pretty JSON, written with `fsutil::write_atomic` after the rename):

```json
{
  "schema": 1,
  "source_md5": "<STREAMINFO MD5 of audio.flac, 32 lowercase hex>",
  "file": "normalized.flac",
  "md5": "<STREAMINFO MD5 of that file>",
  "outcome": "normalized",
  "measured_lufs": -24.0,
  "measured_tp": -24.0,
  "gain_db": 10.0,
  "target": -14.0,
  "ceiling": -1.0
}
```

`file` is `"audio.flac"` for the unchanged outcomes (then `md5 == source_md5`). `measured_lufs` /
`measured_tp` may be `null` (too quiet / silence). The result shown is rebuilt with `result_of`.

### 2.5 Resume never normalizes twice (requirement 7)

- The input of the step is always `audio.flac`, which is never modified after the conversion, and
  `normalized.flac` is never measured or used as an input. So a mix can never be normalized twice, whatever
  happens; at worst the same deterministic computation runs again from the same `audio.flac` and writes the
  same bytes (flacenc is deterministic; checked by a test).
- `resume` accepts the marker only when all of these hold: it parses with `schema == 1`; `target` and `ceiling`
  equal `loudness::TARGET_LUFS` / `CEILING_DBTP`; `outcome` is one of the five names and agrees with `file`
  (`normalized`/`limited` <-> `normalized.flac`, the other three <-> `audio.flac`); the STREAMINFO MD5 of
  `audio.flac` is not all zeros and equals `source_md5`; `file` is a regular file whose STREAMINFO MD5 equals
  `md5`. Only the two FLAC headers are read, so a resume costs milliseconds.
- Crash points: during the write -> a part file and no marker: removed and written again. After the rename and
  before the marker -> `normalized.flac` without a marker: removed and written again (same bytes). An
  `audio.flac` from a build without this feature (no marker) -> normalized now, which is the first time.
- Only URL jobs are ever resumed (local-file job folders are removed as stale at the next start, as today),
  so the acceptance test for resume uses a URL. "Start Over" clears the folder as today.
- A resumed step emits no `normalizing` event (nothing runs) and logs `via=resume`.

### 2.6 Import job changes (`import_job.rs`, `import_batch.rs`)

- `Phase::Normalizing` (serialised `"normalizing"`), between `Preparing` and `Ready`.
- `ImportEvent::Normalizing { progress: f64 }` (0..1). The first one goes through `ctx.phase(...)` (phase change,
  logged `calliope: import job=<j> phase=Normalizing`), the rest through `ctx.progress(...)` (throttled);
  `is_progress` includes it. `JobSnapshot.progress` holds the fraction while normalizing (it is reset by
  `start_extraction` as today).
- `ImportEvent::Ready { metadata, duration_s, loudness: LoudnessResult }`.
- `JobSnapshot.loudness: Option<LoudnessResult>`: set with `ready`, kept through extraction, cancel back to the
  edit pane and a failed extraction; reset to `None` per batch file; `None` before `ready`.
- `BatchOutcome::Saved { track, dropped, loudness: LoudnessResult }` (failed files carry no result; the log has it).
- One helper in `import_job.rs`:
  ```rust
  /// The normalizing step of every import (gui-import-normalize).
  fn normalize_step(ctx: &Ctx, dir: &Path, converted: &Path) -> R<import_normalize::Normalized>
  ```
  It checks cancel; if `import_normalize::resume` gives a result it logs it and returns it; otherwise it calls
  `ctx.phase(Phase::Normalizing, Normalizing { progress: 0.0 })`, runs `import_normalize::normalize` with
  `ctx.cancel.flag()` and a progress closure (sleeps the pace, 2.8, then updates `snap.progress` and sends a
  throttled `Normalizing` event), and maps the outcome: `Err(CANCELLED)` -> `Stop::Cancelled`, any other
  `Err(e)` -> `Stop::Failed { stage: Stage::Prepare, message: "Cannot normalize the audio: {e}" }`. Then it logs
  the result line (2.9).
- `run_url`: after the conversion (or its skip), `let n = normalize_step(ctx, dir, &audio)?;` and `set_ready`
  gets `n.audio` and `n.result`.
- `prepare_file`: after `to_flac`, the same; `Prepared` gains `loudness: LoudnessResult` and its `audio` is
  `n.audio`. `run_file` passes both on. `set_ready` gains a `loudness` parameter and stores it in the snapshot.
- `import_batch.rs` `run_one`: stores `p.loudness` in the snapshot and returns it with the track; `run_batch`
  builds `BatchOutcome::Saved { track, dropped, loudness }` and resets `snap.loudness = None` per file.
- No new `Stage`: a failure of the step is a preparation failure (`stage: prepare`), so it behaves exactly like
  the other preparation failures (requirement 6): the job ends, a local-file job's folder is removed, a URL
  job's folder keeps its download and `audio.flac` for a later resume, the frontend goes back to the source page
  with the message, and a batch records the file as failed and goes on.
- Cancel during the step (requirement 6): the cancel flag stops `measure_mix` / `render_flac` within one block
  (4096 frames, about 0.1 s), `render_flac` removes the part file, and `finish_prepare` ends the job as a
  cancel during preparing does today: back to the source page; a local-file job's folder is removed; a URL job
  keeps its folder (download + `audio.flac`, no part, no `normalized.flac`, no marker). A batch stops and skips
  the rest, as today. Nothing is saved in any case.

### 2.7 Decisions taken by the team (the owner may overrule)

- The copy is 24-bit (2.3).
- `at-ceiling` (gain within 0.1 dB of 0 because of the ceiling) is "not changed" too, with its own text (2.10).
- The resume shows the same line again; the Normalizing step is skipped, not replayed.
- A failed batch file shows only its failure in the summary, not a loudness line.

### 2.8 Pace knob for Cancel tests

`ImportState::with_normalize_pace(self, pace: Duration) -> Self` (builder like `with_rediscovery`; field
`Inner.normalize_pace: Option<Duration>`): the step sleeps `pace` per progress call below 1.0 (about 108 calls
for a 5 s file). The app sets it in `gui.rs` from `CALLIOPE_E2E_IMPORT_PACE=<ms per block>` under
`#[cfg(feature = "e2e-hooks")]`, exactly like `CALLIOPE_E2E_BACKING_PACE`. No effect otherwise.

### 2.9 Log lines

Rust (prefix `calliope: `), one line per step, after the result is known:

```text
import job=<j> normalized outcome=<o> measured_lufs=<x|none> measured_tp=<x|none> gain=<g> lufs=<x|none> tp=<x|none> bits=<16|24> clipped=<n> changed=<b> via=<measure|resume>
```

`measured_*` are the converted file's values, `lufs`/`tp` the values after the gain, `gain` with two decimals
(`fmt_opt` style, `{:.2}`), `bits` the depth of the mix sent (`OUTPUT_BITS` when changed, the converted file's
otherwise), `clipped` 0 for unchanged and resumed. Plus the phase line `import job=<j> phase=Normalizing`.

Frontend (prefix `calliope-ui: `): `import phase=normalizing job=<j>` (the generic phase line), and
`import loudness outcome=<o> gain=<g> lufs=<x|none>` when a `ready` event or an attached snapshot carries a
result (`gain` and `lufs` with one decimal), and `import batch loudness item=<i> outcome=<o> gain=<g>` for each
saved batch item (after its existing `import batch item=... result=saved ...` line, which is not changed).

### 2.10 Frontend

- `src/ui/lib/ipc.ts`: `JobPhase` gains `'normalizing'`; `ImportEvent` gains
  `{ phase: 'normalizing'; progress: number }`; the `ready` event gains `loudness?: LoudnessResult | null`;
  `JobSnapshot` gains `loudness?: LoudnessResult | null`; the saved `BatchItem` gains
  `loudness?: LoudnessResult | null`. Optional in TS so the existing test fixtures stay valid; Rust always sends it.
- `src/ui/lib/loudness-format.ts`: `importLoudnessText(r: LoudnessResult): string` (exact texts, spec
  requirement 4, no final full stop, like the spec's examples):
  - normalized: `Normalized to -14.0 LUFS (gain +6.0 dB)`
  - limited: `Normalized to -16.2 LUFS (gain +3.1 dB, limited by the -1 dBTP peak ceiling)`
  - at-target: `Already at -14.0 LUFS; not changed`
  - at-ceiling: `Already at the -1 dBTP peak ceiling (-15.3 LUFS); not changed`
  - too-quiet: `Too quiet or too short to normalize; imported as is`
  Numbers use the module's existing `num` / `signed` / `whole` helpers (one decimal, never `-0.0`).
- `src/ui/lib/import-state.svelte.ts`: `step()` maps `normalizing` to `'source'` for a single import (like
  `preparing`; a batch stays on `'extract'`); `PHASE_TEXT.normalizing = 'Normalizing the audio...'`;
  `applyEvent`: `normalizing` sets `phase` and `progress`; `ready` stores `j.loudness = e.loudness ?? null`
  and logs the `import loudness` line; `batch-file` clears `j.loudness`; a saved `batch-item` with a result logs
  `import batch loudness`; `attach()` logs `import loudness` when the snapshot has one.
- `SourcePage.svelte`: a `normalizing` block like the `preparing` block: the text "Normalizing the audio...",
  a determinate `Progress` (`value = progress * 100`, `aria-label="Normalizing the audio"`) and the same Cancel.
- `BatchProgress.svelte`: the same block when the current file is normalizing (no Cancel inside: the batch has
  its one Cancel). Its "Done" list: `"{name}: saved as {title}; {text}"` then the existing
  ` (dropped empty stems: a, b)` when any.
- `ImportEditPane.svelte`: a row in the `TrackFields` children after Length: label "Loudness", value
  `importLoudnessText(job.loudness)` (`data-testid="import-loudness"`), only when `job.loudness` is set.
- `ImportView.svelte` (directory summary, "Saved tracks"): `"{title}: {text}"` then `"; dropped empty stems:
  a, b"` when any (today `"{title}: dropped empty stems: ..."`).
- No layout risk: the source page and the edit pane are top-aligned flex columns that scroll; one text row is
  added to each. Checked by screenshots at 1280x800 and 1024x640 (task 7).

### 2.11 Test helpers

- `tests/support/stub-separator`: when the folder `<work dir>/stub-inputs/` exists, every run first copies its
  input to `stub-inputs/NNN.flac` (001, 002, ... in run order). This is "the mix sent for extraction" in the
  tests (headless and GUI), the same idea as the stub tracker's `stub-tracker-last.flac`.
- Test signals with a known loudness are synthesised in the tests (no committed audio): stereo 1 kHz sines
  (EBU Tech 3341 case-1 style: a stereo 1 kHz sine at -23 dBFS peak reads -23.0 LUFS), scaled to an exact
  loudness with the meter (`at_lufs`, as in `tests/acceptance_backing_normalize.rs`), a burst version (a 50 ms
  1 kHz burst at -3 dBFS with 20 ms raised-cosine fades, so the gain is limited by the ceiling), digital
  silence and a 0.3 s tone. They are written as 44.1 kHz stereo FLAC (16- or 24-bit, `flacenc`) so the ffmpeg
  conversion is lossless and "the converted file" equals the fixture sample for sample. GUI tests make theirs
  with the local ffmpeg (`aevalsrc`) into the test's media folder.

## 3. Tasks

Rules for every task:

- Run only the tests the task touches (named below); the lead runs the whole `npm test` once per task before
  committing. Do not run `git commit`.
- `npm` commands run in `src/calliope-gui/`. Rust is formatted with `cargo fmt --all`.
- Tests use temp directories, the real `calliope-stems` on 127.0.0.1 with the stub separator, the fake yt-dlp
  and fixtures only: never a real download, a real dialog, an audio device, the user's repository, another
  host or a server on `0.0.0.0:8765`. No `symphonia`.
- GUI tests: `npm run test:gui -- <filters>` (each binary on its own private display).

### Task 1. `import_normalize.rs`: the step, the marker and resume

- routine: no
- files: `src/calliope-gui/src/import_normalize.rs` (new), `src/calliope-gui/src/import_normalize_tests.rs`
  (new), `src/calliope-gui/src/lib.rs` (`pub mod import_normalize;`, alphabetical).
- does: section 2.3-2.5. The marker struct derives serde `Serialize`/`Deserialize` and is private; MD5s are
  written with `track_meta::md5_hex`. Tests (synthetic 44.1 kHz stereo FLACs written with `flacenc` in a
  `tempfile` dir; helpers may be copied from `backing_normalize_tests.rs`):
  1. a 16-bit tone at -24.00 LUFS: outcome normalized, gain 10.00 +-0.01, `normalized.flac` is 24-bit, decodes
     to `quantise(x * db_to_gain(gain), 24)` sample for sample, measures -14.0 +-0.05 LUFS; marker fields as
     in 2.4; no part file left; `audio.flac` byte-identical to before;
  2. the burst signal: outcome limited, decoded true peak <= -1.0 + 0.01 dBTP, loudness below -14;
  3. a loud signal (-8 LUFS, peaks at full scale): negative gain, no sample of the result at +-full scale;
  4. a tone at -14.00 LUFS: at-target, no `normalized.flac`, `audio == converted`, marker `file` = audio.flac;
  5. digital silence (10 s) and a 0.3 s tone: too-quiet, gain 0, unchanged;
  6. resume: a second `normalize` returns `resumed: true`, never calls `progress`, and `normalized.flac` keeps
     its bytes, inode and mtime;
  7. invalid markers (garbage, schema 2, other target, `source_md5` of another file, `normalized.flac` deleted
     or its MD5 changed, outcome/file mismatch) make `resume` return `None` and `normalize` write again, with
     the same bytes as the first run; a `normalized.flac` without a marker is replaced by the same bytes;
  8. cancel set before the call, and set from the progress callback during measuring and during writing:
     `Err(CANCELLED)`, no part, no `normalized.flac`, no marker, `audio.flac` unchanged;
  9. a leftover part file is replaced; a directory named `.normalized.flac.part` gives an error and leaves
     `audio.flac` untouched; a mono or 8-bit input is refused with a message.
- done when: `cargo test -p calliope-gui --lib import_normalize` passes; `cargo clippy -p calliope-gui
  --all-targets -- -D warnings` and `cargo fmt --all --check` are clean.

### Task 2. Stub separator keeps its inputs

- routine: yes
- files: `src/calliope-gui/tests/support/stub-separator`, `src/calliope-gui/tests/support/README.md`.
- does:
  1. In `stub-separator`, directly after the block that appends to `$STUB_SEPARATOR_LOG` (the `fi` after
     `printf '%s\t%s\t%s\n' ...`), insert exactly:
     ```bash

     # Keep a copy of every input when <work dir>/stub-inputs/ exists (tests check the mix that was sent).
     keep="$PWD/../../stub-inputs"
     if [[ -d $keep ]]; then
       n=$(find "$keep" -maxdepth 1 -name '*.flac' | wc -l)
       cp -- "$in" "$keep/$(printf '%03d' $((n + 1))).flac"
     fi
     ```
  2. In the header comment, after the line `# $STUB_SEPARATOR_LOG, if set, gets the argv (one line, tab separated) appended.`,
     add the line `# If <work dir>/stub-inputs/ exists, each input is copied there as 001.flac, 002.flac, ... in run order.`
  3. In `tests/support/README.md`, at the end of the `stub-separator` section (after its mode table), add the
     paragraph: `If the folder \`stub-inputs/\` exists in the server's work dir, every run first copies its input
     there as \`001.flac\`, \`002.flac\`, ... (run order): the mix the app sent (gui-import-normalize tests).`
- done when: `bash -n tests/support/stub-separator` succeeds, and this check prints `ok` (run in
  `src/calliope-gui/`):
  ```bash
  t=$(mktemp -d) && mkdir -p "$t/work/stub-inputs" "$t/work/jobs/j/out" && cd "$t/work/jobs/j" && \
  STUB_SEPARATOR_MODE=ok "$OLDPWD/tests/support/stub-separator" "$OLDPWD/tests/fixtures/import/untagged.flac" out m >/dev/null && \
  cmp "$t/work/stub-inputs/001.flac" "$OLDPWD/tests/fixtures/import/untagged.flac" && echo ok; cd "$OLDPWD"; rm -rf "$t"
  ```
  and `cargo test -p calliope-stems --test conformance` still passes.

### Task 3. Import job: the Normalizing phase in every import

- routine: no
- files: `src/calliope-gui/src/import_job.rs`, `src/calliope-gui/src/import_batch.rs`,
  `src/calliope-gui/src/import_job_tests.rs`, `src/calliope-gui/src/gui.rs` (pace env), `src/calliope-gui/src/test_server.rs`
  (a helper `TestServer::keep_inputs(&self) -> PathBuf` that creates `work/stub-inputs/` and returns it).
- does: section 2.6, 2.8 and the Rust log line of 2.9. Fix the existing tests that destructure
  `BatchOutcome::Saved { track, dropped }` (add `..`). New tests in `import_job_tests.rs`:
  1. a local audio file: events `Preparing`, `Normalizing` (first with progress 0), then `Ready` with a
     `loudness`; the snapshot carries it; the job's audio is `normalized.flac`; after Extract with keep original
     the stub's `stub-inputs/001.flac` and `original.flac` are byte-identical to the `normalized.flac` seen at
     `ready`;
  2. a video file and a URL (fake yt-dlp): `Normalizing` before `Ready`, the input sent equals the job's
     `normalized.flac`;
  3. URL resume after a "restart" (a second `ImportState` on the same root, `start_url(.., resume=true)`): no
     `Normalizing` event, the same `loudness`, `normalized.flac` and `normalize.json` keep bytes and mtime; and a
     URL folder holding only `audio.flac` (no marker, as an older build left it) is normalized at the resume;
  4. cancel during normalizing (`with_normalize_pace(50 ms)`, wait for phase `Normalizing`, `cancel`): local
     file -> `Cancelled { back_to: Source }`, job folder removed, no track, no staging; URL -> the folder keeps
     download + `audio.flac`, and has no part, `normalized.flac` or marker;
  5. failure: with the pace and a source that needs a gain, create a directory `.normalized.flac.part` in the job folder while it is measuring
     -> `Failed { stage: Prepare }` with a message starting "Cannot normalize the audio:", the job folder is
     removed, no track;
  6. a batch of three generated files (-24 LUFS, -14 LUFS, silence): each file passes `Normalizing` before
     `Uploading`; the items are `Saved` with outcomes normalized / at-target / too-quiet; `snap.loudness` is
     reset between files.
- done when: `cargo test -p calliope-gui --lib import_job` and `cargo test -p calliope-gui --lib import_normalize`
  pass; `cargo test -p calliope-gui --lib ipc` passes (IPC tests that build snapshots); clippy and fmt clean.

### Task 4. Frontend: types, result text and import state

- routine: no
- files: `src/calliope-gui/src/ui/lib/ipc.ts`, `src/calliope-gui/src/ui/lib/loudness-format.ts`,
  `src/calliope-gui/src/ui/lib/loudness-format.test.ts`, `src/calliope-gui/src/ui/lib/import-state.svelte.ts`,
  `src/calliope-gui/src/ui/lib/import-state.test.ts`.
- does: section 2.10 (types, `importLoudnessText`, `step()`, `PHASE_TEXT`, `applyEvent`, `attach`) and the
  frontend log lines of 2.9. Tests: the five texts (incl. `-0.0` never shown, the ceiling `-1` whole);
  `normalizing` keeps a single import on `'source'` and a batch on `'extract'`; footer text; `ready` stores the
  result and logs `import loudness outcome=normalized gain=10.0 lufs=-14.0`; a `ready` without `loudness`
  (old fixtures) stores `null`; `batch-file` clears it; the batch loudness log line.
- done when: `npx vitest run src/ui/lib/loudness-format.test.ts src/ui/lib/import-state.test.ts` passes and
  `npm run check` is clean.

### Task 5. Frontend: the Normalizing step and the result in the Import view

- routine: no
- files: `src/calliope-gui/src/ui/components/import/SourcePage.svelte`,
  `src/calliope-gui/src/ui/components/import/BatchProgress.svelte`,
  `src/calliope-gui/src/ui/components/import/ImportEditPane.svelte`, `src/calliope-gui/src/ui/views/ImportView.svelte`,
  `src/calliope-gui/src/ui/views/ImportView.test.ts`, `src/calliope-gui/src/ui/acceptance_import.test.ts`.
- does: section 2.10 (components). Tests with `mockIPC`: a `normalizing` event shows "Normalizing the audio..."
  with a progressbar at the right value and a Cancel that calls `cancel_import`; `ready` with a result shows
  the Loudness row with the exact text for each outcome; without a result there is no row; the batch progress
  "Done" list and the summary list show `"<name>: saved as <title>; <text>"` and `"<title>: <text>"`, with
  dropped stems after it.
- done when: `npx vitest run src/ui/views/ImportView.test.ts src/ui/acceptance_import.test.ts` passes,
  `npm run check` is clean, `npm run build` succeeds and `cargo test -p calliope-gui --test frontend` passes
  (no static `style=`, CSP rules).

### Task 6. Headless acceptance tests

- routine: no
- files: `src/calliope-gui/tests/acceptance_import_normalize.rs` (new).
- does: one test per acceptance criterion (eight, named as in section 7), with its own rig built like `tests/acceptance_existing_track_import.rs`
  (temp repository and sources, `RealServer` with the stub separator in `ok` mode and a `stub-inputs/` folder in
  its work dir, the real local ffmpeg/ffprobe, the fake yt-dlp for the URL). Signals per 2.11. Every test also
  checks that the sources and an "outside" folder are unchanged and that no `*.part` is left in `import-tmp`.
  The "mix sent" is `stub-inputs/NNN.flac`, decoded with claxon and measured with `loudness::Meter`.
- done when: `cargo build -p calliope-stems && cargo test -p calliope-gui --test acceptance_import_normalize`
  passes.

### Task 7. GUI tests and the UI guide

- routine: no
- files: `src/calliope-gui/tests/gui_import_e2e.rs`, `src/calliope-gui/tests/common/mod.rs` (a
  `start_import_app_with(dirs, stub, keep_original, answers, env: &[(&str, &str)])` variant passing extra env
  to the app; a hook in the import `Rig` to create files in the media folder before its snapshot),
  `docs/ui.md`.
- does: three `#[ignore]` tests (names unique across binaries):
  1. `import_normalize_result_shown`: a local FLAC at about -24 LUFS (ffmpeg `aevalsrc=0.0631*sin(2*PI*1000*t)`
     on both channels, 44.1 kHz, 10 s) with keep original on: waits for `import phase=normalizing` and
     `import loudness outcome=normalized` (gain 9.8..10.2), screenshots the edit pane (`import-normalize-ready`,
     at 1280x800 and 1024x640: the Loudness row is visible and nothing overlaps), Extract, saved; the new track's
     `original.flac` is byte-identical to `stems-work/stub-inputs/001.flac`; `track.json` has no loudness field.
  2. `import_normalize_cancel`: `CALLIOPE_E2E_IMPORT_PACE=50`; Extract, wait `phase=normalizing`, screenshot the
     source page showing "Normalizing the audio..." (`import-normalizing`), click Cancel, wait
     `phase=cancelled`; `check_disk(None)` (no new track, no `import-tmp` folder left).
  3. `import_directory_normalize_summary`: a folder with three generated files (-24 LUFS, -14 LUFS, silence);
     waits for `batch finished saved=3 failed=0` and three `import batch loudness` lines with outcomes
     normalized, at-target, too-quiet; screenshot `import-batch-normalize-summary` shows the three result texts.
  Look at the screenshots. In `docs/ui.md` "Decided by the team", add a `(gui-import-normalize)` entry: the
  Normalizing step on the source page and in the batch progress (text, determinate bar, Cancel), the Loudness
  row in the edit pane, the five texts, the batch list formats.
- done when: `npm run test:gui -- import_normalize import_directory_normalize_summary` passes, the existing
  `npm run test:gui -- gui_import_e2e` passes, and the screenshots were looked at.

### Task 8. README

- routine: yes
- files: `src/calliope-gui/README.md` (ASCII only).
- does: in the section "## Import and stem extraction", after the numbered list of steps and before the line
  `Needs:`, add exactly this paragraph (followed by one blank line):
  ```
  Every import first brings the song to the same loudness as the backing tracks: -14 LUFS integrated
  (ITU-R BS.1770), with the true peak at most -1 dBTP and no limiter. The step is shown as "Normalizing the
  audio..." and its result appears in the track details before you press Extract, for example "Normalized to
  -14.0 LUFS (gain +6.0 dB)", "Normalized to -16.2 LUFS (gain +3.1 dB, limited by the -1 dBTP peak ceiling)",
  "Already at -14.0 LUFS; not changed" or "Too quiet or too short to normalize; imported as is". A directory
  import lists the result of each file. The stems are made from the normalized mix, and the kept original
  (original.flac) is the normalized mix. The result is not stored in track.json.
  ```
  In the same section, replace `(downloads, converted audio)` with
  `(downloads, converted audio, the normalized mix normalized.flac and its normalize.json)`, and in the
  "Keep the original mix" bullet replace `the converted source as` with `the normalized mix as`.
- done when: `grep -n "Normalizing the audio" src/calliope-gui/README.md` finds the paragraph,
  `grep -c "normalize.json" src/calliope-gui/README.md` prints 1, and
  `LC_ALL=C grep -nP '[^\x00-\x7F]' src/calliope-gui/README.md` prints nothing.

## 4. Test strategy

- Unit (Rust, `--lib`): `import_normalize_tests` (the step on synthetic FLACs: gains, limits, untouched cases,
  24-bit output, sample-exact scaling, peaks, marker validity, deterministic rewrite, cancel, leftovers);
  `import_job_tests` (event order, every source, resume, cancel, failure, batch) against the real
  `calliope-stems` with the stub separator on 127.0.0.1 and the fake yt-dlp.
- Acceptance (Rust, headless): `tests/acceptance_import_normalize.rs`, one test per criterion.
- Frontend (vitest + jsdom, IPC faked with `mockIPC`): texts, state, the Normalizing block, the Loudness row,
  the batch lists.
- GUI (display, `npm run test:gui`): the result line and the Normalizing step on screen, Cancel during the step
  (paced with `CALLIOPE_E2E_IMPORT_PACE`), the directory summary; app and server inside `unshare -rn`.
- Stand-ins: the edge-AI server is the real `calliope-stems` binary with `tests/support/stub-separator` (now
  also keeping its inputs); YouTube is `tests/support/bin/yt-dlp`; audio is synthesised in the tests (no
  committed audio, no downloads); ffmpeg/ffprobe are the local tools.
- Whole suite: `npm test` in `src/calliope-gui/` (headless, all crates). GUI: `npm run test:gui`.

## 5. Deployment

Laptop only. `calliope-gui` is rebuilt (`npm run build:app`). The `calliope-stems` server on archserver is not
changed and needs no redeploy; it receives a 24-bit FLAC more often than before (16-bit lossless sources),
which it already accepts.

## 6. Manual checks (owner, on the laptop, with the real server on archserver)

1. Quiet song: Import > Stem Extraction > Local Audio File, pick an old, quiet CD rip (FLAC). The source page
   shows "Normalizing the audio..." with a bar, then the track details show "Normalized to -14.0 LUFS (gain
   +x.x dB)". Extract with "keep original" on. In the Player, the track's "original" plays at about the level
   of a newer track's original.
2. Loud master from a URL: the line shows a negative gain ("gain -x.x dB"). The stems sound normal (no
   distortion).
3. Very dynamic piece (classical or a live recording with loud peaks): the line says "limited by the -1 dBTP
   peak ceiling" and shows a level below -14 LUFS.
4. Directory import of an album folder: the progress shows "Normalizing the audio..." for each file and the
   summary lists one result per saved track.
5. Cancel: start a long (10 min or more) local file and press Cancel while "Normalizing the audio..." shows.
   The source page comes back, nothing new is in the Library, and `<repository>/import-tmp/` holds no `file-...`
   folder.
6. Resume: import a URL, close Calliope in the track details, start it again and import the same URL, choose
   Resume. The track details come back quickly, without the Normalizing step, with the same result line.
7. Timing: note how long "Normalizing the audio..." takes for a 10-15 minute track (expected well under a
   minute on the laptop).

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests |
|---|---|---|
| -24 LUFS file: mix sent -14.0 +-0.1 LUFS, line "gain +10.0 dB" | 1, 3, 4, 6 | `acceptance_import_normalize::ac1_quiet_file_is_sent_at_minus_14`; `import_normalize_tests` case 1; `loudness-format.test.ts`; GUI `import_normalize_result_shown` |
| Gain limited by the peak: TP <= -1 dBTP, below -14 LUFS, "limited" line | 1, 3, 4, 6 | `ac2_peaks_limit_the_gain`; `import_normalize_tests` cases 2, 3 |
| Already at -14.0: sample-identical, "not changed" | 1, 3, 4, 6 | `ac3_a_file_at_the_target_is_sent_unchanged`; `import_normalize_tests` case 4 |
| Silence or under 0.4 s: goes on as converted, "Too quiet or too short" | 1, 3, 4, 6 | `ac4_silence_and_short_files_are_imported_as_is`; `import_normalize_tests` case 5 |
| Keep original: `original.flac` measures as the mix sent | 3, 6, 7 | `ac5_the_kept_original_is_the_mix_sent`; `import_job_tests` 1; GUI `import_normalize_result_shown` |
| URL, video and directory: normalized before extraction, summary per file | 3, 5, 6, 7 | `ac6_url_video_and_directory_are_normalized_first`; `import_job_tests` 2, 6; `ImportView.test.ts`; GUI `import_directory_normalize_summary` |
| Cancel during Normalizing: nothing saved, temp files removed | 1, 3, 5, 6, 7 | `ac7_cancel_while_normalizing_leaves_nothing`; `import_normalize_tests` case 8; `import_job_tests` 4; GUI `import_normalize_cancel` |
| Resume after a restart: not normalized again, same bytes | 1, 3, 6 | `ac8_a_resumed_import_is_not_normalized_again` (URL; a second `ImportState` on the same root); `import_normalize_tests` cases 6, 7; `import_job_tests` 3 |

Requirement 6's failure half (a failed step fails the import with its reason and leaves nothing): `import_job_tests` 5
and `import_normalize_tests` case 9. Requirement 8 (logged): the Rust line of 2.9, asserted in the GUI tests from the
app's stderr (`normalized outcome=normalized measured_lufs=` with a value near -24).

## 8. Open questions

None. The spec's questions were answered by the owner on 2026-11-18; the team decisions in 2.7 are recorded in
`docs/architecture.md` and can be overruled.
