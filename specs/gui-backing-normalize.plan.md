# Plan: gui-backing-normalize (normalize backing tracks to -14 LUFS, ceiling -1 dBTP)

Spec: `specs/gui-backing-normalize.md` (owner accepted the defaults of its Open questions on 2026-10-28: -14 LUFS,
-1 dBTP ceiling without a limiter, the variant shown in "Save as", fixed values, plain backing tracks out of scope).
Base: `903a903` on `spec/gui-backing-normalize`. Paths follow "Paths in this document" of `docs/architecture.md`:
`src/*.rs`, `src/ui/...` and `tests/...` are inside `src/calliope-gui/`.

## 1. Summary

The Editor's Save measures the mix it is about to write (ITU-R BS.1770-4 integrated loudness and true peak) in a first
pass over the stems. It then writes the backing with one constant gain folded into the stem gains, before clipping and
quantising, so the file is at -14 LUFS, or quieter when that gain would push the true peak above -1 dBTP. A new
**Normalize** button next to Save does the same for the existing file of the variant shown in "Save as". It reads that
FLAC, measures it, writes a scaled copy to a part file and swaps it in atomically. The old file goes to the repository
trash, and an open Player session of the track is released so it reopens the new file. The variant's entry in
`track.json` records the loudness, the true peak and the gain (`backings[].loudness`, additive). The meter is the
MIT-licensed `ebur128` crate behind a small `loudness.rs` module.

## 2. Design

### 2.1 What already exists and is reused

- **Save pipeline** (`backing_render.rs`): `render_flac(input, part, cancel, progress)` opens one `StemReader` per
  checked stem (full precision, interleaved stereo f32, mono copied to both channels), mixes blocks with `mixer::mix`
  (sum × gains, hard clip, clipped count), quantises with `mixer::quantise` (16 bits, or 24 if a stem is), encodes with
  `flacenc` and writes the part file `.calliope-backing-<uuid>.part` in the track folder (`create_new`, fsync). The save
  job (`spawn`/`run`) reports `saving` progress (throttled to 10/s), commits with `Repository::save_backing` under the
  repository lock, and sends exactly one final event. `flacenc` already keeps the whole encoded stream in memory before
  writing it (about 0.5-0.7 of the PCM size). This feature keeps that.
- **`flacenc` writes the STREAMINFO MD5** of the audio (`stream.stream_info().md5_digest()`). `claxon` exposes it on
  read (`StreamInfo::md5sum`). Both are used to tie a loudness record to the exact audio.
- **Repository writes**: `save_backing` (stems-list conflict check, trash of the replaced file in
  `trash/<stamp>-<id>-backing/`, `link_no_clobber`, `write_atomic` of `track.json`, undo on failure), `FileTrash`,
  `fsutil::write_atomic`, `sync_dir`. The repository lock (`RepoLock`, `locked`) is the app's single writer mutex.
- **Player hand-over**: `PlayerManager::release(id)` flushes, stops and closes the session and emits `closed`. The
  Player view then reopens the track the next time it is shown (`player-state.svelte.ts`, `case 'closed'`, `pl.reopen++`).
  Reopening decodes the source again (`player_source`) and recomputes the waveform peaks. `PlayerTempoStore` runs its
  file write inside `PlayerHost::with` right after `release`, so the Player cannot reopen in between.
- **The Editor job slot**: `EditorManager.save: Option<SaveRun>`, `SaveShared` (cancel, progress, finished, outcome),
  `reap`, `cancel_save`, `shutdown_save` (app exit), `decorate` (progress in the snapshot).
- **Frontend**: `MixLane.svelte` (Save row: Save button, then the progress group with Cancel, or the "Save as" text;
  one reserved two-line message slot), `editor-state.svelte.ts` (`save`, `cancelSave`, events, `saveMessage`), `ipc.ts`.

### 2.2 The meter: the `ebur128` crate behind `src/loudness.rs` (requirement 1)

**Choice: the `ebur128` crate**, version `0.1.10` pinned exactly (`=0.1.10`; the lead chose it on 2026-10-28 over 0.1.11, released only two days before) (MIT, Sebastian Dröge; a pure-Rust port of libebur128, the meter used
by GStreamer and ffmpeg's `ebur128` filter family). Checked on 2026-10-28:

- **Licence and maintenance.** MIT. 0.1.11 was released on 2026-10-26, 0.1.10 on 2024-10-26, with steady releases since
  2020 and about 1.1 M downloads. MSRV 1.60.
- **Dependencies.** `bitflags 1`, `smallvec 1`, `dasp_frame 0.11` and `dasp_sample 0.11`. All are MIT or Apache-2.0 and
  all are already in `Cargo.lock` (via `signalsmith-stretch` and Tauri), so no new transitive crates come in. No C code
  and no build script in normal builds (`cc` is only for its optional C test feature).
- **Not symphonia.** It has no decoder at all.
- **Coverage.** It does what BS.1770-4 asks:
  - K-weighting coefficients computed for any sample rate. They are the BS.1770 analogue prototype through a bilinear
    transform, so they are right at 22.05, 44.1 and 48 kHz.
  - 400 ms blocks with 75 % overlap.
  - The absolute -70 LUFS gate and the relative -10 LU gate.
  - Channel weights: L/R 1.0, surround 1.41.
  - True peak: 4x oversampling below 96 kHz, a 48-tap Hann-windowed sinc polyphase interpolator. `true_peak()` returns
    max(oversampled, sample) peak, so the true peak is never below the sample peak.
- **EBU tests.** Its README says it passes all EBU Tech 3341 and 3342 tests. Its own test suite uses ±0.1 LU and the EBU
  true-peak tolerance (+0.2/-0.4 dB).

Accuracy against our stricter 0.1 dB true-peak criterion was checked before deciding:

- **The interpolator.** Its taps were evaluated numerically in a scratch script (our own code; nothing external was run).
  - The per-phase gain is within 0.04 dB up to 0.3·fs, and within 0.002 dB at DC.
  - On synthesised Tech 3341 true-peak tones with 20 ms raised-cosine fades, the error is at most 0.05 dB:
    cases 15-19 at 48 and 44.1 kHz give -6.02 / -6.05 / -6.01 / -6.02 / +2.95 dBTP for -6.02 / -6.02 / -6.02 / -6.02 /
    +2.98.
  - Without fades, the abrupt start of a tone makes any band-limited interpolator overshoot (up to +0.65 dB for the
    fs/8 case). That is a property of the test signal, not of the meter, which is why our tones are faded (§4).
- **The K-weighting.** Its formula, evaluated the same way, gives a stereo 1 kHz sine at -23 dBFS peak:
  - -22.96 LUFS at 22.05 kHz;
  - -22.99 LUFS at 44.1 kHz;
  - -22.99 LUFS at 48 kHz.

Alternatives considered:

- **Our own BS.1770 in Rust** (biquads, gating and a polyphase FIR in about 300 lines). It is feasible, but it means
  re-deriving and re-validating what a widely used, EBU-tested and maintained MIT crate already does. The team would own
  the bugs, and there is no gain in licence or size.
- **ffmpeg's `loudnorm` / `ebur128` filters.** They would make the Editor depend on ffmpeg (it deliberately doesn't),
  need text parsing, and still need our own gain and atomic-write logic.
- **`symphonia`.** Forbidden, and not a meter anyway.

`src/loudness.rs` (new, pure, no IO) is the only module that names `ebur128`, so it can be replaced behind this
interface:

```rust
pub const TARGET_LUFS: f64 = -14.0;
pub const CEILING_DBTP: f64 = -1.0;
/// "Already normalized" tolerance, in LU (and dB for the ceiling).
pub const TOLERANCE: f64 = 0.1;
/// Below this the gated loudness is undefined (BS.1770 absolute gate); shown in the "too quiet" text.
pub const GATE_LUFS: f64 = -70.0;

/// BS.1770-4 meter for interleaved stereo f32 (values beyond ±1 allowed: the unclipped mix).
pub struct Meter { /* ebur128::EbuR128, Mode::I | Mode::TRUE_PEAK */ }
impl Meter {
    pub fn new(sample_rate: u32) -> Result<Meter, String>;   // 2 channels, Left/Right
    pub fn add(&mut self, interleaved: &[f32]) -> Result<(), String>;
    pub fn finish(&self) -> Measure;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measure {
    /// Integrated loudness, LUFS; None when no block passes the gates (silence, below -70 LUFS, or under 400 ms).
    pub lufs: Option<f64>,
    /// Max true peak over both channels, dBTP; None for digital silence.
    pub true_peak: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome { Normalized, Limited, AtTarget, AtCeiling, TooQuiet }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision { pub outcome: Outcome, pub gain_db: f64 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose { Save, Normalize }

pub fn decide(m: Measure, purpose: Purpose) -> Decision;
/// The one place a dB gain becomes the linear factor (f64 maths, then `as f32`); Save, Normalize and the tests use it.
pub fn db_to_gain(db: f64) -> f32;

/// What the UI shows (IPC payload): the file's values after the operation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoudnessResult {
    pub outcome: Outcome,
    pub lufs: Option<f64>,
    pub true_peak: Option<f64>,
    pub gain_db: f64,
    pub target: f64,
    pub ceiling: f64,
}
pub fn result_of(m: Measure, d: Decision) -> LoudnessResult; // lufs/true_peak = measured + gain_db (unchanged when gain 0)
pub fn record_of(m: Measure, d: Decision, md5: [u8; 16]) -> LoudnessRecord; // track.json entry, §2.5
```

`Meter::new` errors only for rates the crate refuses (it accepts any rate ≥ 1 Hz; true peak works below 192 kHz). The
Save rates are the stems' rates (22.05-96 kHz in practice).

### 2.3 The gain rule and the 0.1 LU rule (requirements 2, 3, 5)

With `L` = measured integrated loudness and `TP` = measured true peak:

- `g_t = TARGET_LUFS - L` is the gain that reaches the target.
- `g_c = CEILING_DBTP - TP` is the largest gain that keeps the true peak at or below the ceiling.
- `g = min(g_t, g_c)` is the gain applied. It may be negative: a loud or clipping mix is turned down.

`decide(m, purpose)`:

1. `L` or `TP` is None → `TooQuiet`, gain 0. The file is written as mixed (Save) or left alone (Normalize).
2. `purpose == Normalize` and `|g| <= TOLERANCE + 1e-9` → left untouched, gain 0. The outcome is `AtTarget` when
   `g_t <= g_c` (the loudness is the binding limit), else `AtCeiling`. This one rule covers both cases of
   requirement 5:
   - a file within 0.1 LU of the target, with its true peak not more than 0.1 dB above the ceiling;
   - a quieter file whose true peak is already within 0.1 dB of the ceiling.

   A file at the target whose true peak is more than 0.1 dB above the ceiling (an old clipped backing) is not left
   alone: it is turned down to the ceiling.
3. Otherwise gain `g`, outcome `Limited` when `g_c < g_t`, else `Normalized`.

Save always applies `g`, even a tiny one: the file is being written anyway, so there is no "untouched" case. Values are
kept unrounded (f64) in memory, events and `track.json`. Only the UI rounds them (one decimal).

**No limiter, no compression.** With `g <= g_c` the scaled true peak is at most -1 dBTP. Since the true peak is never
below the sample peak, every sample is at most -1 dBFS. So the clip stage of `mixer::mix` never fires after
normalization (requirement 3, acceptance criterion 4). The clipped count is still computed and reported (it would show
if something were wrong).

**Quantisation.** The gain is applied in f32 before `mixer::quantise`, so the written file's loudness and true peak
equal `L + g` and `TP + g` up to the rounding of the last bit (well under 0.01 dB at 16 bits). No dither, as before.

### 2.4 Save: two passes over the stems (requirement 3)

Memory decides the method:

- **Rendering the mix to float in memory and then scaling it** would hold 15 min × 60 × 48 000 × 2 × 4 B ≈ 690 MB of
  float PCM on top of the encoded stream. That is rejected.
- **Two streaming passes over the stem files** hold only the readers' blocks. The second decode costs time, not memory:
  `claxon` runs at opt-level 3, and decoding 6 stems of 15 minutes takes seconds.

Changes in `backing_render.rs`:

- `mixer::sum(inputs, gains, out)` is the summing loop of `mixer::mix` without the clip. `mix` becomes `sum` + clip with
  identical results (Task 2).
- `open_readers(input) -> Result<Vec<(StemReader, f32)>, String>` is factored out of `render_inner`, with the same rate
  and depth checks and messages. Stems with gain 0 are skipped, as now.
- `pub fn measure_mix(input: &RenderInput, cancel: &AtomicBool, progress: &mut dyn FnMut(f32)) -> Result<Measure, String>`:
  - reads blocks of 4096 frames, `mixer::sum`s them (unclipped) and feeds the `Meter`;
  - stops after `input.frames` frames;
  - calls `progress` per block with `done / total`;
  - returns `Err(CANCELLED)` when `cancel` is set.
- `RenderStats` gains `md5: [u8; 16]` (from the encoded stream's STREAMINFO).
- **The save job's `run`**:
  1. Pass 1: `measure_mix(&p.input)`, progress mapped to 0..0.4.
  2. `d = decide(m, Purpose::Save)`; `G = db_to_gain(d.gain_db)`.
  3. Pass 2: `render_flac` on a copy of the input whose stem gains are `s.gain * G` (f32 product), progress mapped to
     0.4..1.0. Folding `G` into the stem gains keeps `render_flac`, `mixer::mix` and their exact-sample tests unchanged.
     Tests rebuild the expected samples with the same products.
  4. Logs `calliope: editor measured id=<id> job=save lufs=<x|none> tp=<x|none>` after pass 1.
  5. Commits through the Player gate (§2.8) and the repository lock: `save_backing(BackingSave { .., loudness:
     Some(record_of(m, d, stats.md5)) })`.
  6. `Saved` carries `loudness: result_of(m, d)`. The `editor saved` log line gains
     ` outcome=<o> lufs=<x.xx|none> tp=<x.xx|none> gain=<x.xx>` at its end.
- **Cancel**: the existing per-block checks in both passes; the part file is removed as before.
- **Test pace**: `SaveParams.pace: Option<Duration>` is slept in the job's progress closure, so it applies to every
  block of both passes (§2.10).

### 2.5 `track.json`: `backings[].loudness` (requirement 6)

Each variant may carry:

```json
"loudness": {"lufs": -14.0, "true_peak": -4.02, "gain_db": 6.013, "target": -14.0, "ceiling": -1.0,
             "md5": "9e107d9d372bb6826bd81d3542a419d6"}
```

- **`lufs` and `true_peak`** are the values of the file as it is now, after the gain (null for "too quiet").
- **`gain_db`** is the gain the last write applied: relative to the raw mix for Save, to the previous file for Normalize,
  and 0 for a record-only update.
- **`target` and `ceiling`** are the rule's values when the record was written.
- **`md5`** is the STREAMINFO MD5 of the audio, 32 lowercase hex digits.
- **Additive, no schema bump**, as with `exports` and `position`. Older builds ignore the unknown key on read
  (`BackingVariant` has no `deny_unknown_fields`), so they keep reading the file (requirement 6).
  - When an older build rewrites `track.json` (a Library edit or its own Save), its `BackingVariant` drops the key.
  - That is harmless: the record is only a shortcut. Without it, Normalize measures the file. An older build's Save
    writes an un-normalized file, and dropping the record is then correct.
- **Rust**: `track_meta::LoudnessRecord { lufs: Option<f64>, true_peak: Option<f64>, gain_db: f64, target: f64,
  ceiling: f64, md5: String }` and `BackingVariant.loudness: Option<LoudnessRecord>`.
  - The field is `#[serde(default, deserialize_with = "lenient_loudness", skip_serializing_if = "Option::is_none")]`.
  - **Lenient read**: anything that is not an object with finite numbers in range makes the whole record `None`, never a
    track problem. The ranges are lufs -200..=20 or null, true_peak -200..=40 or null, gain_db -200..=200, target and
    ceiling -100..=0, and md5 32 lowercase hex digits.
  - **Strict write**: the record is only built by `loudness::record_of`.
- **The revision**: it includes `backings`, so a record change changes the revision exactly like a Save does. The Editor
  already reloads the library after `saved` and does the same after `normalized`.

### 2.6 Normalize (requirement 4): `src/backing_normalize.rs` (new)

The job runs on its own thread, built like the save job (`catch_unwind`, one final event, part cleanup), and shares
`SaveShared`, `Throttle`, `measure_mix`, `render_flac` and `CANCELLED` with `backing_render`.

```rust
pub struct NormalizeParams {
    pub id: String, pub dir: PathBuf,
    pub variant: String,          // the session's VariantTarget.id (the one shown in "Save as")
    pub file: String,             // its `backings/<x>.flac`
    pub repo: Repository, pub lock: Arc<dyn RepoLock>, pub gate: Arc<dyn PlayerGate>,
    pub pace: Option<Duration>,
}
pub fn spawn(p: NormalizeParams, shared: Arc<SaveShared>, sink: EventSink) -> JoinHandle<()>;
```

Steps:

1. **Check the file.**
   - Take `before = fsutil::FileStamp::of(dir/file)`: `symlink_metadata`, a regular file only (len, mtime, and on unix
     dev + ino). A missing file, a symlink or a folder fails with "The backing file is missing; save the backing first".
   - Read the variant from `track.json`. A missing variant or a different `file` fails with "conflict: the backing
     variant changed on disk; select the track again".
2. **Open the file** with `StemReader::open`.
   - It must be stereo (`StemReader::channels()`, new accessor). Otherwise: "Only stereo backings can be normalized".
     Save always writes stereo, so only hand-made files hit this.
   - The rate comes from the file, the bits from the file (16 or 24; `StemReader` already refuses others), and the frames
     from STREAMINFO `samples`. When that is unknown or 0: "The backing file has no length in its header; save it
     again".
   - Read the MD5 with `StemReader::md5()`, a new accessor returning `StreamInfo::md5sum`.
3. **Shortcut through the record.** The record is used when all of these hold:
   - it exists;
   - its `target` and `ceiling` equal the constants;
   - its `md5` equals the file's (and is not all zeros);
   - its `lufs` and `true_peak` are numbers;
   - `decide(Measure{those}, Normalize)` is `AtTarget` or `AtCeiling`.

   Then the job ends at once with `changed: false`, and nothing is read or written (`via=record`). Otherwise it
   measures. The record never sets a gain.
4. **Measure** (progress 0..0.4): `measure_mix` over `RenderInput { stems: [RenderStem { path: dir/file, gain: 1.0 }],
   sample_rate, bits, frames }`. Then `d = decide(m, Purpose::Normalize)`.
5. **Untouched** (`AtTarget`, `AtCeiling`, `TooQuiet`): the audio file is not written.
   - When the stored record differs from `record_of(m, d, md5)`, the job writes only the record, under the repository
     lock and without the Player gate: `Repository::record_backing_loudness`, §2.7.
   - `changed: false`; `track` is the re-read record when `track.json` was written, else null.
6. **Write** (progress 0.4..1.0): `render_flac` over the same input with gain `db_to_gain(d.gain_db)` into
   `.calliope-backing-<uuid>.part` in the track folder. The output has the same rate, bits, channels and frame count.
7. **Commit**: `gate.without_player(id, || locked(lock, || repo.replace_backing_audio(BackingReplace { .. })))`, §2.7
   and §2.8. `changed: true`.
8. **Final event** `Normalized { id, file, loudness: result_of(m, d), changed, track, warnings }`. Log
   `calliope: editor normalized id=<id> file=<f> outcome=<o> lufs=<x.xx|none> tp=<x.xx|none> gain=<x.xx> changed=<b>
   via=<record|measure>`.

**Cancel** is checked per block in both passes and between the steps.

- Once step 7 has started, cancel is too late, as with Save. The commit takes milliseconds.
- On cancel or failure, the part file is removed and nothing else changes. The old file was never opened for writing,
  so it stays byte-identical (constraint, acceptance criterion 7).
- The final event is `SaveCancelled { job: normalize }` or `SaveFailed { job: normalize, message }`. Logs:
  `calliope: editor normalize cancelled id=<id>` and `calliope: editor normalize failed id=<id> message=<m>`.

**What it never touches**: the stems, `player.json` (the Player's own flush writes its own state, unchanged in content),
the variant's `mix`, other variants, `audio` and `original`. The tempo map, the first beat and the loops therefore stay
as they are (requirement 7).

### 2.7 Repository commits (`repository.rs`, `fsutil.rs`)

- **`BackingSave` gains `loudness: Option<LoudnessRecord>`.** `save_backing` stores it on the created or replaced
  variant, and nothing else changes. Existing tests pass `None`.
- **`pub struct BackingReplace<'a> { id, variant: &'a str, file: &'a str, part: &'a Path, before: FileStamp, loudness:
  LoudnessRecord }`** and **`pub fn replace_backing_audio(&self, r) -> Result<SaveResult, String>`**:
  1. Load the track (a problem → error).
  2. Check the variant: the id must exist with the same `file`. Otherwise:
     "conflict: the backing variant changed on disk; select the track again".
  3. Check the file: `FileStamp::of(dest)` must equal `before`. Otherwise:
     "conflict: the backing file changed on disk while normalizing; nothing was changed".
  4. Check the part: it must be in the track folder and be a regular file.
  5. **Trash the old file without removing it.** `FileTrash::new(root, stamp, id, "backing").link_in(dest)` is a new
     `FileTrash` method. It hard-links `dest` into the trash folder, falls back to a copy where links are not supported,
     and returns the trash path. Its error is "could not keep the old file in the trash (nothing was changed)".
  6. **`fs::rename(part, dest)`**, then `sync_dir`. The rename replaces the file atomically: readers see either the whole
     old file or the whole new one. When it fails, the trash link and the trash folder it created are removed (an empty
     folder only), and the error is "cannot put the normalized file in place (nothing was changed)".
  7. **Update `track.json`**: `variant.loudness = Some(rec)`, `variant.modified = now`, `meta.modified = now`, then
     `validate_for_read` and `write_atomic`. When that fails, `fs::rename(trash_path, dest)` puts the old inode back
     atomically, the now-empty trash folder is removed, and the error is
     "cannot write track.json (nothing was changed)".
  8. Return `SaveResult { track: load_record(id), warnings }`.
- **`pub fn record_backing_loudness(&self, id, variant, file, before: FileStamp, rec) -> Result<TrackRecord, String>`**:
  the same checks 1-3, then only `variant.loudness = Some(rec)` and `write_atomic`. `modified` is not touched, because
  the audio did not change.
- **`fsutil::FileStamp { len, modified: Option<SystemTime>, dev, ino }`** with `FileStamp::of(path) -> io::Result<..>`
  (a regular file, never a symlink; `dev`/`ino` come from `MetadataExt` on unix and are 0 elsewhere), plus
  `FileTrash::link_in`.

Why hard-link + rename rather than Save's "move to trash, then link": the destination name never disappears, so a crash
or a failure at any point leaves either the old or the new file under the name (constraint "replaced atomically").
Moving the old file into the trash keeps the rule "Calliope never deletes user files". Normalizing a file twice gives two
trash entries, as two Saves do.

### 2.8 The Player follows (requirement 7, acceptance criterion 9)

```rust
// backing_render.rs
pub trait PlayerGate: Send + Sync {
    /// Runs `f` while no Player session holds track `id`: an open one is flushed (when writable) and closed first,
    /// and its `closed` event makes the Player view reopen it. An error closing it refuses `f`.
    fn without_player(&self, id: &str, f: &mut dyn FnMut() -> Result<(), String>) -> Result<(), String>;
}
pub struct NoPlayer; // runs f directly (default; tests)
```

- **`PlayerManager::release_for_audio(id) -> Result<bool, String>`** (new). It is `release` without the
  newer-`player.json` refusal:
  - `try_flush` already skips read-only sessions;
  - a failed flush of a writable one returns "The Player could not save its state: <e>; the backing was not changed";
  - it also cancels a pending open of the same id (as `close` does with `pending`);
  - it logs `calliope: player released id=<id> reason=backing` and emits `Closed`.
- **The app's gate** `PlayerFileGate { host: Arc<PlayerHost> }` in `player_ipc.rs`: `host.with(|m| {
  m.release_for_audio(id)?; f() })`. The file swap runs under the `PlayerHost` lock, so the Player cannot reopen the old
  file between the release and the rename.
- **Lock order** is PlayerHost, then the repository lock, the same as `PlayerTempoStore::replace` followed by the
  write. No code takes the repository lock and then the PlayerHost lock; the task checks this with a grep. `f` never
  touches the `EditorState` lock.
- **Wiring.** The manager holds the gate: `EditorManager::set_player_gate(Arc<dyn PlayerGate>)`, default `NoPlayer`.
  `gui.rs` sets the `PlayerFileGate` at setup. So no `start_save` call site changes.
- **Save uses the gate too** (team decision). Until now a Save left an open Player session playing the old PCM. The
  Player is only released at the commit, so it plays on during the measuring and writing.
- **Effects of the reopen:**
  - the Player's waveform is recomputed from the new file;
  - the export reads the file at export time, so it uses the new one;
  - an export already running keeps the old inode it opened, which stays valid in the trash;
  - the Editor's Mix waveform shows the stems' mix at the lane gains and is unchanged, because it never showed the
    file.

### 2.9 Editor manager and IPC (`editor.rs`, `ipc.rs`)

- **Job kinds.** `SaveRun` gains `kind: JobKind`, where `#[serde(rename_all = "lowercase")] pub enum JobKind { Save,
  Normalize }`. One job at a time:
  - `start_save` refuses while either kind runs, with the existing "A save is already running", or "Normalize is
    running" during a normalize;
  - `cancel_save` and `shutdown_save` cover both kinds.
- **`start_normalize(id, repo, lock) -> Result<EditorSnapshot, String>`** refuses with:
  - no session: the existing error;
  - a job running: "A save is already running" or "Normalize is already running";
  - a detection running: "Wait for the beat detection to finish";
  - `!variant.exists` or the file missing now: "Save the backing first".

  It spawns `backing_normalize::spawn` with the manager's gate and pace.
- **Snapshot**: `EditorSnapshot.job: Option<JobKind>` is set while a job runs; `saving` stays its progress.
- **Events** (additive, kebab-case `kind` as today):
  - `saving { id, progress, job }`;
  - `saved { .., loudness: LoudnessResult }`;
  - new `normalized { id, file, loudness: LoudnessResult, changed: bool, track: TrackRecord | null, warnings }`;
  - `save-failed { id, message, job }` and `save-cancelled { id, job }`.
- **`reap`**: a finished normalize leaves the session's variant as it is (`exists` stays true).
- **Command `normalize_backing(id) -> EditorSnapshot`** (async, `spawn_blocking`, like `save_backing`). It is
  registered in `generate_handler!` (`src/gui.rs`), `COMMANDS` (`build.rs`) and `capabilities/main.json`. Cancel reuses
  `cancel_backing_save(id)`, which stops whichever job runs.
- **Test knobs:**
  - `EditorManager::set_job_pace(Option<Duration>)` (`#[doc(hidden)] pub`) is passed into both jobs' params.
  - In `e2e-hooks` builds, `gui.rs` reads `CALLIOPE_E2E_BACKING_PACE=<ms per block>` (parsed as u64, > 0) and calls it,
    like `CALLIOPE_E2E_EXPORT_PACE`.

### 2.10 Frontend

**`ipc.ts`**:

- types `JobKind = 'save' | 'normalize'`, `LoudnessOutcome = 'normalized' | 'limited' | 'at-target' | 'at-ceiling' |
  'too-quiet'`, and `LoudnessResult { outcome, lufs: number | null, true_peak: number | null, gain_db, target,
  ceiling }`;
- `EditorSnapshot.job?: JobKind | null`;
- the event union gets the fields of §2.9: `saving.job?`, `saved.loudness?`, `save-failed.job?`, `save-cancelled.job?`
  and the new `normalized`. Optional on the TS side, so the existing mocks stay valid;
- the wrapper `normalizeBacking(id)`.

**`src/ui/lib/loudness-format.ts`** (new, pure) holds `loudnessText(r: LoudnessResult, job: JobKind): string`:

- Numbers use one decimal with an ASCII minus. The gain is signed (`+3.2`, `-1.0`, `+0.0`, never `-0.0`). The ceiling is
  shown without decimals when it is whole (`-1`).
- Save:
  - normalized: `Saved, normalized to -14.0 LUFS (gain +3.2 dB).`
  - limited: `Saved at -15.6 LUFS (gain -1.0 dB, limited by the -1 dBTP peak ceiling).`
  - too-quiet: `Saved. Too quiet or too short to normalize.`
  - (at-target and at-ceiling do not happen on Save; they read like normalized.)
- Normalize:
  - normalized: `Normalized to -14.0 LUFS (gain +3.2 dB).`
  - limited: `Normalized to -15.6 LUFS (gain -1.0 dB, limited by the -1 dBTP peak ceiling).`
  - at-target: `Already at the target (-14.0 LUFS); the file was not changed.`
  - at-ceiling: `Already at the -1 dBTP peak ceiling (-15.6 LUFS); the file was not changed.`
  - too-quiet: `Too quiet or too short to normalize; the file was not changed.`

**`editor-state.svelte.ts`**:

- `ed.job: JobKind | null` comes from the snapshot and the events and is cleared with `ed.saving`.
- `canNormalize() = status active && ed.variant?.exists && ed.saving === null && ed.detect === null`.
- `normalize()` mirrors `save()`: it clears the messages, sets `ed.saving = 0` and `ed.job = 'normalize'`, logs
  `editor normalize start` and calls `normalizeBacking`.
- `cancelSave()` is unchanged; it cancels either job.
- **`saved`**:
  - The message is `loudnessText(e.loudness, 'save')` when `loudness` is present, else `Saved.` (old mocks).
  - The existing clipped sentence ("N samples were clipped (about x s); lower some volumes and save again.") is still
    appended when `clipped_samples > 0`, and the warnings after it.
  - The log line becomes `editor saved id=<id> file=<f> clipped=<n> outcome=<o|none> gain=<g.g|none>`, with the new
    fields at the end.
- **`normalized`**:
  - The message is `loudnessText(e.loudness, 'normalize')` plus the warnings.
  - The log is `editor normalized id=<id> outcome=<o> gain=<g.g> changed=<b>`.
  - `void load()` runs when `e.track` is not null.
- **`save-cancelled`**: `Normalize cancelled; the file was not changed.` when `job === 'normalize'`, else
  `Save cancelled.`. **`save-failed`**: the message as today.

**`MixLane.svelte`, the Save row.** Its height stays 3.25 rem and no row is added:

```
[ Save | Cancel ] [ Normalize | Cancel ] [ progress bar ··· 42% ]   (while a job runs)
[ Save ]          [ Normalize ]          [ Save as: Backing (backings/backing.flac) ]   (idle)
```

- Save stays `h-[3.25rem] w-28` at the same place, so the GUI tests' `SAVE` point does not move.
- Normalize is `variant="outline" h-[3.25rem] w-[7.5rem] shrink-0 text-lg`. It is disabled by `!canNormalize()`, with
  the title "Save the backing first" when the variant has no file.
- **The running job's own button reads "Cancel"** and stays enabled (it calls `cancelSave`); the other button is
  disabled. This is the Autodetect BPM / Detect BPM pattern. The separate Cancel button of the progress group is removed.
- The reason is width. At 1024x640 the Editor pane is about 380 px wide (docs/ui.md). Save 112 + Normalize 120 + two
  gaps leaves about 124 px. That is enough for a shrinking progress bar and the `w-12` percent, but not for a third
  ~100 px Cancel button.
- The progress `aria-label` is "Saving progress" or "Normalizing progress".
- The two-line message slot is unchanged. The longest line, at about 80 characters, wraps to two lines at 380 px and to
  one at 1280 px.

**Layout log.** `MixLane` logs `editor layout mix=<lane offsetHeight px> overflow=<pane scrollHeight - clientHeight px>
width=<lane px>`:

- after mount and after the size settles (`ResizeObserver`, debounced to the next animation frame);
- and once after each `saveMessage` change.

GUI tests compare `mix` before and after the results (it must stay equal) and read `overflow`.

### 2.11 Logs (added to `docs/architecture.md` "Frontend log lines" and the Rust lines)

| Side | Lines |
|---|---|
| Frontend | `editor normalize start` |
| Frontend | `editor normalized id=<id> outcome=<o> gain=<g> changed=<b>` |
| Frontend | `editor saved id= file= clipped= outcome=<o\|none> gain=<g\|none>` (fields added at the end) |
| Frontend | `editor layout mix=<px> overflow=<px> width=<px>` |
| Rust | `calliope: editor measured id=<id> job=<save\|normalize> lufs=<x.xx\|none> tp=<x.xx\|none>` |
| Rust | `calliope: editor saved id= file= bits= clipped= outcome=<o> lufs=<..> tp=<..> gain=<..>` (fields added at the end) |
| Rust | `calliope: editor normalized id=<id> file=<f> outcome=<o> lufs=<..> tp=<..> gain=<..> changed=<b> via=<record\|measure>` |
| Rust | `calliope: editor normalize failed id=<id> message=<m>` |
| Rust | `calliope: editor normalize cancelled id=<id>` |
| Rust | `calliope: player released id=<id> reason=backing` |

## 3. Tasks

Every task follows these rules:

- Run only the tests the task touches (named below). The lead runs `npm test` once per task before committing.
- `npm` commands run in `src/calliope-gui/`. Rust is formatted with `cargo fmt --all`.
- Tests never open a real audio device or a real dialog. They never reach `0.0.0.0:8765` or another host; this feature
  needs no server at all. They use temporary repositories only.
- No `symphonia`. Test signals are synthesised in the tests; nothing is downloaded.

### Task 1. Add the `ebur128` dependency
- routine: yes
- files: `src/calliope-gui/Cargo.toml`, `Cargo.toml` (workspace root), `docs/licences.md`.
- does:
  1. In `src/calliope-gui/Cargo.toml` `[dependencies]`, after the `flacenc` line, add exactly
     `ebur128 = "=0.1.10"`.
  2. In the root `Cargo.toml`, after the `[profile.dev.package.flacenc]` block, add:
     ```
     [profile.dev.package.ebur128]
     opt-level = 3
     ```
     Also change the comment above the claxon block to "Decoding/encoding FLAC and measuring loudness are slow at
     opt-level 0; keep these hot crates optimised in dev and test builds."
  3. In `docs/licences.md`, in the "Rust crates of calliope-gui" table, add a row after `flacenc`:
     `| \`ebur128\` | dependency | \`=0.1.10\` | <locked version from Cargo.lock> | MIT | loudness (ITU-R BS.1770-4 / EBU R128) and true peak of backing tracks |`.
     Read the licence from `cargo metadata --format-version 1 | python3 -c 'import json,sys;
     print([p["license"] for p in json.load(sys.stdin)["packages"] if p["name"]=="ebur128"])'`. If it is not `MIT`,
     stop and report.
  4. Do not run `git commit`.
- done when:
  - `cargo build -p calliope-gui` succeeds (it fetches the crate once);
  - `cargo test -p calliope-gui --test frontend licence_record_lists_every_direct_dependency` passes;
  - `npm run build` succeeds, and `cargo test -p calliope-gui --test frontend
    dist_ships_third_party_notices_for_every_linked_crate` passes;
  - `Cargo.lock` gained exactly one new package (`ebur128`), checked with `git diff --stat Cargo.lock` and
    `git diff Cargo.lock | grep '^+name'`.

### Task 2. `mixer::sum`
- routine: yes
- files: `src/mixer.rs`.
- does:
  1. Add, above `pub fn mix`:
     ```rust
     /// Sums interleaved-stereo blocks with their gains into `out` without clipping (the loudness
     /// measurement sees the mix as it would be before the clip). Each input must be at least `out.len()` long.
     pub fn sum(inputs: &[&[f32]], gains: &[f32], out: &mut [f32]) {
     ```
     Its body is exactly the current first part of `mix`: `out.fill(0.0)` and the loop over inputs and gains.
  2. Make `mix` call `sum(inputs, gains, out);` and then keep only its clip loop and `clipped` return.
  3. Add a test `sum_does_not_clip`: `a = [0.75f32, -0.75]`, `b = [0.5f32, -0.5]`, gains `[1.0, 1.0]` give
     `[1.25, -1.25]`; `mix` of the same gives `[1.0, -1.0]` with 2 clipped.
  4. Do not run `git commit`.
- done when: `cargo test -p calliope-gui --lib mixer` passes (all old tests unchanged plus the new one);
  `cargo clippy -p calliope-gui --all-targets -- -D warnings` is clean; `cargo fmt --all --check` is clean.

### Task 3. `loudness.rs`: meter, gain rule, results, and the EBU reference tests
- routine: no
- files: `src/loudness.rs` (new), `src/loudness_tests.rs` (new), `src/lib.rs` (`pub mod loudness;`).
- does: §2.2 and §2.3. Constants, `Meter` (`ebur128::EbuR128::new(2, rate, Mode::I | Mode::TRUE_PEAK)`), `Measure`,
  `Outcome`, `Decision`, `Purpose`, `decide`, `db_to_gain`, `LoudnessResult` and `result_of`. (`record_of` needs
  `track_meta::LoudnessRecord` and comes in Task 4.) In `finish`:
  - `loudness_global()` of `-inf` or `< GATE_LUFS` gives `lufs: None`;
  - the true peak is the max of `true_peak(0)` and `true_peak(1)`, linear, converted to dB, and 0 gives `None`.
- tests (`loudness_tests.rs`). Signals are synthesised with f64 maths. Each tone has a 20 ms raised-cosine fade-in and
  fade-out, except the loudness cases where the fades do not matter. The cases are stereo, both channels equal unless
  stated:
  - **EBU Tech 3341** (loudness, ±0.1 LU), each at 48 000 and 44 100 Hz, and case 1 also at 22 050 Hz (the fixtures'
    rate):
    - case 1: 1 kHz sine at -23 dBFS peak for 20 s → -23.0;
    - case 2: -33 dBFS, 20 s → -33.0;
    - case 3: -36 / -23 / -36 dBFS for 10 / 60 / 10 s → -23.0;
    - case 4: -72 / -36 / -23 / -36 / -72 dBFS for 10 / 10 / 60 / 10 / 10 s → -23.0, which exercises both gates;
    - case 5: -26 / -20 / -26 dBFS for 20 / 20.1 / 20 s → -23.0.
  - **Channel weights**: case 1 in the left channel only → -26.0 ± 0.1 LUFS.
  - **EBU Tech 3341 true peak** (±0.1 dB), at 48 000 and 44 100 Hz, 3 s each, with fades:
    - case 15: sine at fs/4, 0.5 FS, phase 0° → -6.0 dBTP;
    - case 16: fs/4, 0.5 FS, 45° → -6.0;
    - case 17: fs/6, 0.5 FS, 60° → -6.0;
    - case 18: fs/8, 0.5 FS, 67.5° → -6.0;
    - case 19: fs/4, 1.41 FS, 45° → +3.0 (f32 input above 1.0).
  - **Tech 3342** is loudness range, which this feature does not measure; no LRA test.
  - **Too quiet**: digital silence gives `lufs: None, true_peak: None`; a -80 dBFS tone gives `lufs: None` with a
    true peak; 300 ms of a -20 dBFS tone gives `lufs: None` (no complete 400 ms block).
  - **Block size**: feeding case 1 in blocks of 4096 frames or in one call gives the same values within 1e-9.
  - **`decide` table**, with `(L, TP) → outcome, gain`:
    - (-20, -10) Save → Normalized +6.0;
    - (-20, -3) → Limited +2.0;
    - (-14.05, -4) Normalize → AtTarget 0;
    - (-14.05, -4) Save → Normalized +0.05;
    - (-15.6, -1.05) Normalize → AtCeiling 0;
    - (-14.0, +0.5) Normalize → Limited -1.5;
    - (-10, -0.5) → Normalized -4.0 (since g_t = -4 < g_c = -0.5);
    - (None, _) → TooQuiet 0;
    - exactly 0.1 away (L = -14.1, TP = -10) Normalize → AtTarget, and 0.11 away → Normalized +0.11.
  - `db_to_gain(6.0)` is 1.995262 (±1e-6), and `db_to_gain(0.0) == 1.0` exactly.
  - `result_of` adds the gain to both values and keeps None as None.
- done when: `cargo test -p calliope-gui --lib loudness` passes; clippy clean. If a reference case misses its
  tolerance, do not widen it: report the measured values to the lead (the fallback in §2.2 is then decided).

### Task 4. `LoudnessRecord` in `track_meta`; `StemReader::channels` / `md5`
- routine: no
- files: `src/track_meta.rs`, `src/stem_audio.rs`, `src/loudness.rs` (`record_of`), `src/loudness_tests.rs`.
- does:
  - §2.5: `LoudnessRecord`, the field on `BackingVariant`, the lenient deserializer and the ranges, and `md5_hex([u8;16])
    -> String`. Existing `BackingVariant { .. }` literals in the crate get `loudness: None`. Check with
    `grep -rn "BackingVariant {" src tests`.
  - `loudness::record_of(m, d, md5) -> LoudnessRecord`: values after the gain, `target` and `ceiling` from the
    constants.
  - `StemReader::channels() -> u32` and `StemReader::md5() -> [u8; 16]`, both read from the STREAMINFO kept at open.
- tests:
  - `track_meta`: a variant with a valid record round-trips byte-stably through `to_json_pretty`;
  - a variant without one has no `loudness` key on write;
  - each bad record makes `loudness` `None` and the track still loads with no problem. The bad cases are a string, NaN
    given as a string, lufs 50, a short md5, upper-case md5 and a missing `gain_db`;
  - unknown keys inside the record are dropped;
  - `stem_audio`: `channels`/`md5` of a flacenc-written fixture equal claxon's STREAMINFO;
  - `record_of` of (-20, -10, +6) gives lufs -14, true_peak -4.
- done when: `cargo test -p calliope-gui --lib track_meta stem_audio loudness` passes; clippy clean.

### Task 5. Repository commits: `replace_backing_audio`, `record_backing_loudness`, `FileStamp`, `link_in`
- routine: no
- files: `src/fsutil.rs`, `src/repository.rs`, `src/repository_tests.rs`.
- does: §2.7. It also adds `loudness` to `BackingSave` (the existing literals in `repository_tests.rs` and
  `backing_render.rs` pass `None`; Task 6 sets it).
- tests (`repository_tests.rs`, temp repositories, the outside-folder check as in the existing tests):
  - A replace puts the part's bytes under the name, and the old bytes are in `trash/<stamp>-<id>-backing/backing.flac`
    (same content). `track.json` has the record and new `modified` values, and the other variants, `mix`, the stems,
    `exports` and unknown fields are kept.
  - A changed file (rewritten after `before`) or a changed variant `file` is a `conflict:` error. The part is left for
    the job to remove, and the old file and `track.json` are byte-identical.
  - A `track.json` write failure restores the old file. Use a read-only track folder (`chmod 0555`, skipped as root)
    after the link step via a test hook, or an injected failing writer. The test asserts the old bytes are under the
    name, `track.json` is unchanged and no empty trash folder is left.
  - `record_backing_loudness` writes only `loudness`, and `modified` is unchanged.
  - A symlink at the backing's place is refused by `FileStamp::of`.
  - `link_in` falls back to a copy when `hard_link` fails. Inject the failure through a `#[cfg(test)]` switch in
    `fsutil`.
- done when: `cargo test -p calliope-gui --lib repository fsutil` passes; clippy clean.

### Task 6. Save normalizes: two passes, the gate, the pace
- routine: no
- files: `src/backing_render.rs`, `src/backing_render_tests.rs`, `src/editor.rs`, `src/editor_tests.rs`,
  `tests/acceptance_backing_editor.rs`.
- does:
  - §2.4: `open_readers`, `measure_mix`, `RenderStats.md5`, the two-pass `run`, the `loudness` on `Saved` and in the log;
  - §2.8: the `PlayerGate` trait and `NoPlayer`, and the commit inside `without_player`;
  - `SaveParams` gains `gate` and `pace`;
  - `EditorManager::set_player_gate` and `set_job_pace` (only stored here; the normalize use comes in Task 8).
- tests:
  - `backing_render_tests.rs`:
    - `measure_mix` of two stems equals a `Meter` fed with `mixer::sum` of the same blocks;
    - it is not clipped (a +12 dB mix measures a true peak above 0 dBTP);
    - its progress is monotonic and below 1.0;
    - a cancel returns `CANCELLED`;
    - `RenderStats.md5` equals claxon's STREAMINFO MD5 of the written file;
    - the existing render tests pass unchanged.
  - `editor_tests.rs`, the save test at about line 685: the expected samples become
    `quantise((sv*(gv*G) + sd*(gd*G)).clamp(-1, 1))` with `G = loudness::db_to_gain(loudness.gain_db)` from the
    `Saved` event, in exactly that f32 order. `clipped_samples` is 0 and the outcome is `normalized` or `limited`.
  - `acceptance_backing_editor.rs` `ac16_ac17_...`: the same change to `want`. Also assert
    `meta["backings"][0]["loudness"]["target"] == -14.0`, and that the decoded file measures -14.0 ± 0.1 LUFS (or the
    limited value) with `loudness::Meter`.
  - A gate test: a recording `PlayerGate` is called exactly once per Save with the track id, and a gate that returns Err
    makes the save fail with that message, with no part left and the old file and `track.json` unchanged.
- done when: `cargo test -p calliope-gui --lib backing_render editor_tests mixer` and `cargo test -p calliope-gui --test
  acceptance_backing_editor --test acceptance_beat_autodetect` pass; clippy clean.

### Task 7. The Normalize job (`backing_normalize.rs`)
- routine: no
- files: `src/backing_normalize.rs` (new), `src/backing_normalize_tests.rs` (new), `src/lib.rs`.
- does: §2.6 (all steps, messages, logs, cancel, pace, the record shortcut).
- tests, on temp folders with a hand-made `track.json` and FLACs written with `flacenc` (the helper of
  `backing_render_tests.rs` can be copied):
  - **Tone at -20 LUFS** (a 1 kHz stereo sine at -20 dBFS peak, 22 050 Hz, 16 bit, 6 s):
    - the result is `normalized`, gain +6.0 ± 0.1;
    - the file measures -14.0 ± 0.1 LUFS with the same frame count, rate, bits and channels;
    - every sample equals `quantise(old * G)`;
    - the old bytes are in the trash and the record is in `track.json`.
  - **24-bit input** stays 24-bit.
  - **A high-crest signal** (the tone plus a 2 ms burst to -0.5 dBFS) gives `limited`. The new true peak is -1.0 ± 0.1
    and the loudness is below -14.
  - **A second run** returns `at-target` through the record (`via=record`): the file and `track.json` bytes and mtimes
    are unchanged.
  - **With the record removed**, the same file gives `at-target` by measuring (`via=measure`), the audio is unchanged,
    and only the record is written.
  - **A record whose md5 does not match** is ignored.
  - **Silence** gives `too-quiet`, with the file unchanged.
  - **Cancel**, with a pace of 5 ms per block and the cancel set from the sink at the first progress ≥ 0.5, so it lands
    in the write pass:
    - `save-cancelled { job: normalize }`;
    - the file is byte-identical, `track.json` is byte-identical, there is no part file and no trash entry.
  - **Errors**: a mono file, a missing file, a symlink and a variant whose file changed meanwhile each give their
    message and leave nothing behind.
  - **The gate** is called only when the file changes.
- done when: `cargo test -p calliope-gui --lib backing_normalize` passes; clippy clean.

### Task 8. Player release and the Editor/IPC wiring
- routine: no
- files: `src/player.rs`, `src/player_tests.rs`, `src/player_ipc.rs`, `src/editor.rs`, `src/editor_tests.rs`,
  `src/ipc.rs`, `src/gui.rs`, `build.rs`, `capabilities/main.json`.
- does:
  - §2.8: `release_for_audio` and `PlayerFileGate`. Grep that no code takes the repository lock and then the
    PlayerHost lock.
  - §2.9: `JobKind`, `start_normalize`, the snapshot `job` field, event fields, `reap`, refusals, `normalize_backing`
    (`do_normalize_backing` body + command), registration in the three places, and the e2e pace env in `gui.rs`.
  - `gui.rs`: `editor.set_player_gate(Arc::new(PlayerFileGate::new(host.clone())))`. `host` is created after `editor`
    today, so reorder setup so the host exists first (keep the `before_play` hook working).
- tests:
  - `player_tests.rs`:
    - `release_for_audio` on an open session flushes a dirty position, emits `Closed` and logs `reason=backing`;
    - on a read-only (newer `player.json`) session it closes without writing and the file is unchanged;
    - on another id it returns `Ok(false)`;
    - a pending open of the id is cancelled.
  - `editor_tests.rs`:
    - the refusals of `start_normalize` (no file, detection running via `test_fake_detect`, job running);
    - `snapshot.job` while running;
    - `cancel_save` stops a normalize;
    - `shutdown_save` cancels it within the cap and leaves no part.
  - `tests/frontend.rs`'s static command check passes with the new command.
- done when: `cargo test -p calliope-gui --lib player_tests editor_tests ipc` and `cargo test -p calliope-gui --test
  frontend` pass; clippy clean.

### Task 9. Frontend: IPC types and the result texts
- routine: no
- files: `src/ui/lib/ipc.ts`, `src/ui/lib/loudness-format.ts` (new), `src/ui/lib/loudness-format.test.ts` (new),
  `src/ui/lib/ipc-editor.test.ts`.
- does: the `ipc.ts` part of §2.10 and `loudnessText`.
- tests:
  - every outcome × job text of §2.10, exactly;
  - `-0.04` gain → `+0.0`;
  - ceiling `-1` → `-1`, `-1.5` → `-1.5`;
  - `ipc-editor.test.ts`: `normalizeBacking` invokes `normalize_backing` with `{ id }`.
- done when: `npm run check` and `npx vitest run src/ui/lib/loudness-format.test.ts src/ui/lib/ipc-editor.test.ts` pass.

### Task 10. Frontend: Editor state and the Normalize button
- routine: no
- files: `src/ui/lib/editor-state.svelte.ts`, `src/ui/lib/editor-state.test.ts`,
  `src/ui/components/editor/MixLane.svelte`, `src/ui/components/editor/EditorPane.test.ts`,
  `src/ui/acceptance_editor.test.ts`.
- does: the `editor-state` and `MixLane` parts of §2.10, including the Cancel swap, the disabled title and the layout
  log.
- tests (vitest + mockIPC):
  - `canNormalize` is false without a file, while saving, while normalizing and while detecting;
  - `normalize()` calls the command and shows the progress;
  - while a job runs the Save button reads Cancel (save) or the Normalize button reads Cancel (normalize), and clicking
    it calls `cancel_backing_save`;
  - the `normalized`, `saved` (with and without `loudness`), `save-cancelled { job }` and `save-failed` messages;
  - the log lines;
  - an old `saved` mock without `loudness` still shows "Saved.";
  - update the existing tests that click the progress group's Cancel button. They now click the Save button, which
    reads Cancel.
- done when: `npm run check` and `npx vitest run src/ui/lib/editor-state.test.ts
  src/ui/components/editor/EditorPane.test.ts src/ui/acceptance_editor.test.ts` pass.

### Task 11. Headless acceptance tests
- routine: no
- files: `tests/acceptance_backing_normalize.rs` (new).
- does: one test per acceptance criterion, named `ac1_..` to `ac9_..`. They run on a temp copy of
  `tests/fixtures/library-editor` (or hand-made temp tracks) with `EditorManager` + `ManualBackend` + `NoPlayer`, or the
  gate of AC9. Stems are synthesised at test time with flacenc, at known levels. The mapping is in §7.
  - **AC9** uses a `PlayerManager` (`ManualBackend`) behind a test gate that calls `release_for_audio` under its own
    mutex. The test opens the Player on `backing:backing`, normalizes, and checks the `Closed` event. It then reopens the
    Player, pulls samples through the `ManualBackend`, and compares them with the new file's decode (bit-exact at 100 %
    speed).
  - **AC5 and AC7** also check that the stems' bytes, `player.json` (tempo, loops, `found_ms` as JSON) and the variant's
    `mix` are unchanged.
- done when: `cargo test -p calliope-gui --test acceptance_backing_normalize` passes; clippy clean.

### Task 12. GUI tests: Normalize in the Editor, the clipped Save, the Player reopen
- routine: no
- files: `tests/gui_editor_e2e.rs`, `tests/common/mod.rs` (only if a helper is needed, e.g. a capture-WAV RMS helper
  that does not exist yet).
- does:
  1. `states_and_lanes`, the second Save at +12 dB: assert `clipped=0` and `outcome=limited` on the `editor saved` line,
     no `clipped` in the frontend line's message, and the screenshot `editor-saved-limited` (replaces
     `editor-saved-clipped`). The trash and old-file assertions stay.
  2. New `editor_normalize`, on `ED_MIXED` (fixture 202 "Mixed Before": a 330 Hz backing at the ffmpeg sine default
     amplitude of 1/8, about -18 LUFS):
     1. Select it. Assert the `editor layout` line, `overflow=0` at 1280x800, and the `mix=` height.
     2. Press Normalize. Wait for `calliope: editor normalized` and assert `outcome=normalized`, gain about +4
        (`field` parsed, ±1), and `changed=true`.
     3. Check the file: decode it and check RMS ratio = gain ± 0.1 dB against the old bytes saved before. Check
        `track.json` `loudness.target == -14`, the stems' bytes unchanged, and the screenshot `editor-normalized`.
     4. Press Normalize again: `outcome=at-target` and `changed=false`. The file's bytes and mtime are unchanged. Take
        the screenshot `editor-normalize-already`.
     5. **Cancel.** Overwrite the backing with the old bytes again, restart the app with
        `CALLIOPE_E2E_BACKING_PACE=40`, press Normalize, wait for `editor normalize start` plus 1 s, then click the same
        place (now Cancel). Wait for `calliope: editor normalize cancelled` and check the file is byte-identical. Take
        the screenshot `editor-normalize-running` before the click.
     6. **Disabled.** Select `ED_FOUR` (no backing file): a click on Normalize logs no `editor normalize start` within
        700 ms. Take the screenshot `editor-normalize-disabled`.
     7. **Layout.** The `mix=` value is the same after each result line as before. Repeat step 1's check at 1024x640
        (`i3-msg` resize, as the other tests do) with a result line showing, and take the screenshot
        `editor-normalize-1024`.
  3. New `editor_normalize_player_reopen`:
     1. Open `ED_MIXED` in the Player (Alt+5), play 1 s with the capture backend, then pause.
     2. Go to the Editor and Normalize. Wait for `calliope: player released id=… reason=backing`.
     3. Go back to the Player: `player active` again. Play 1 s and compare the 330 Hz tone power in the two capture
        segments. The ratio must be the logged gain ± 0.3 dB.
     4. Assert the `player.json` loops and tempo are unchanged.
  4. Measure `NORMALIZE` (centre of the new button) on a screenshot of the floated 1280x800 window, and add it next to
     `SAVE`. No other constant changes, because the lane's height does not change; verify `MIX_PLAY`, `TIME` and `SAVE`
     still hit by running `states_and_lanes`.
- done when: `npm run test:gui -- states_and_lanes editor_normalize editor_normalize_player_reopen` passes, and the
  screenshots in `target/gui-shots/` have been looked at:
  - the button row is on one line;
  - Cancel is in the Normalize button's place;
  - the result line fits in the slot without overlap;
  - nothing is cut off at 1024x640.

### Task 13. Docs: README and UI guide
- routine: no (edits inside two long files; content checked by reading)
- files: `src/calliope-gui/README.md` (the "Editor" section; ASCII only), `docs/ui.md`.
- does:
  - **README**: Save writes the backing normalized to -14 LUFS (integrated, ITU-R BS.1770 / EBU R128), and quieter when
    the -1 dBTP true-peak ceiling stops the gain; no limiter. Normalize does the same for an existing backing file. The
    result line, Cancel, and the trash entry are described. A track open in the Player is closed and reopened.
  - **`docs/ui.md`**: a "(gui-backing-normalize)" entry under the Editor layout entries. Save row: Save, Normalize, then
    the target or the progress; the running job's button becomes Cancel; Normalize is disabled without a file, during a
    job and during a detection; the result texts of §2.10; no row added.
  - Check README ASCII with `grep -nP '[^\x00-\x7F]' src/calliope-gui/README.md` (no output).
- done when: `cargo test -p calliope-gui --test frontend` passes (static doc checks, if any) and the grep prints nothing.

## 4. Test strategy

- **Meter** (`loudness_tests.rs`):
  - the synthesised EBU Tech 3341 cases 1-5 (loudness, ±0.1 LU) at 48 and 44.1 kHz, and case 1 at 22.05 kHz;
  - cases 15-19 (true peak, ±0.1 dB), faded tones, at 48 and 44.1 kHz;
  - the left-only channel-weight case, silence / too quiet / too short, and block-size independence.

  No external test files are downloaded. Tech 3342 (loudness range) is not needed, because no LRA is measured.
- **Rules**: the `decide` table, including the exact 0.1 boundary, `db_to_gain` and `result_of` / `record_of`.
- **Files**: `track_meta` (lenient record, byte-stable round trip) and `repository_tests` (atomic replace, trash link,
  conflicts, the restore on a failed `track.json` write, record-only writes, the outside-folder check).
- **Jobs**:
  - `backing_render_tests` (`measure_mix`, md5, old exact tests unchanged);
  - `backing_normalize_tests` (every outcome; the record shortcut; cancel during the write pass with a 5 ms pace;
    errors);
  - `editor_tests` and `acceptance_backing_editor` (Save's exact samples with the folded gain).

  Files are always decoded with claxon and compared sample by sample.
- **Player**: `player_tests` (`release_for_audio`). AC9 headless uses `ManualBackend` samples after the reopen (bit-exact
  to the new file). GUI uses the capture backend's tone power before and after.
- **Frontend**: vitest for `loudness-format`, `editor-state` and the `MixLane` / `EditorPane` components with mockIPC.
- **GUI on a display**: `gui_editor_e2e` `states_and_lanes` (the clipped Save is now limited, with no clip),
  `editor_normalize` and `editor_normalize_player_reopen`. They use the capturing `NullBackend` and temp XDG dirs, assert
  on the log lines, on the files and on `editor layout`, and take screenshots that the agent looks at. Cancel uses
  `CALLIOPE_E2E_BACKING_PACE=40` (ms per 4096-frame block): fixture 202 is 6 s at 22.05 kHz, about 33 blocks per pass,
  so about 2.6 s in total.
- **Hardware and network**: none involved. No server, no real audio device (the e2e builds only have `NullBackend`;
  unit tests use `ManualBackend`), no real dialogs, temp repositories only.
- **Whole headless suite**: `cd src/calliope-gui && npm test`. **GUI suite**: `cd src/calliope-gui && npm run test:gui`
  (or `DISPLAY=:1 npm run test:gui:serial`).

## 5. Deployment

Only calliope-gui changes. It runs on the laptop: `npm run build:app` in `src/calliope-gui/`, then
`target/release/calliope-gui`. The new crate is compiled in; there is no new system package and no ffmpeg change.
calliope-stems on archserver is not touched and needs no redeploy. Existing repositories need no migration. Old backings
stay as they are until the user presses Normalize or saves again.

## 6. Manual checks (owner, on the laptop)

1. **Save normalizes.**
   1. Open a stem track in the Editor and set a mix.
   2. Press Save. The line should read "Saved, normalized to -14.0 LUFS (gain ...)" or "Saved at ... LUFS (gain ...,
      limited by the -1 dBTP peak ceiling)".
   3. Listen in the Player. It should sound like the Editor's mix, only louder or quieter as a whole: no pumping, no
      distortion.
2. **A hot mix.** Push some stems to +12 dB until CLIP lights while playing in the Editor, then Save. The line should say
   "limited by the -1 dBTP peak ceiling" with no clipped-samples warning. The saved backing should not crackle in the
   Player.
3. **Normalize an old backing.**
   1. Pick a track whose backing was saved before this version.
   2. Press Normalize. The progress bar should run and the Normalize button should read Cancel.
   3. The line should read "Normalized to -14.0 LUFS (gain ...)".
   4. Press Normalize again. It should say "Already at the target (-14.0 LUFS); the file was not changed".
4. **Cancel.** On a long track (8-15 minutes) press Normalize, then Cancel at once. The line should read "Normalize
   cancelled; the file was not changed", and the Player should still play the old level.
5. **Player follows.**
   1. Play the track in the Player, then go to the Editor and Normalize.
   2. Back in the Player, the track should reopen at the same position with the same loops and tempo sections, and play
      at the new level.
6. **Same level across tracks.** Normalize three or four backings, then switch between them in the Player and on the
   RC-5 after an export. They should need no volume change. Very dynamic tracks may still sound quieter: those are the
   ones the ceiling limited.
7. **Optional cross-check.** Run `ffmpeg -nostdin -i backings/backing.flac -af ebur128=peak=true -f null -` on a
   normalized file. "I:" should be -14.0 ± 0.1 LUFS (or the value shown when limited), and "Peak:" should be at most
   -1.0 dBFS (true peak).
8. **Layout.**
   1. At 1280x800, Save, Normalize and "Save as: ..." should be on one line, and the Mix lane should not grow when a
      result appears.
   2. At the smallest window (1024x640), the row should still fit; the bar is shorter.
9. **Trash.** After a Normalize the repository's `trash/` has a `<stamp>-<id>-backing/backing.flac` with the old file.

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests |
|---|---|---|
| EBU reference signals within 0.1 LU / 0.1 dB | 1, 3 | `loudness_tests` Tech 3341 cases 1-5 (3 rates), 15-19 (2 rates), channel weights; `acceptance_backing_normalize::ac1_ebu_reference_signals` (re-runs cases 1, 3, 4, 16, 19 through `measure_mix` on FLAC files written at 16 and 24 bits) |
| Mix at -20 LUFS / -10 dBTP saved → -14.0 ± 0.1, line says +6.0 dB | 3, 6, 9, 10, 11 | `decide` table; `ac2_..` (stem tone at -20 LUFS with a -10 dBFS burst; decoded file -14.0 ± 0.1; `loudness.gain_db` 6.0 ± 0.1; `loudnessText` gives "gain +6.0 dB"); `loudness-format.test.ts`; `editor-state.test.ts` |
| -14 gain would exceed -1 dBTP → peak -1 ± 0.1, loudness below -14, line says limited | 3, 6, 9, 10, 11, 12 | `decide` table; `backing_normalize_tests` high-crest; `ac3_..` (decoded true peak -1.0 ± 0.1, lufs < -14, outcome `limited`, text contains "limited by the -1 dBTP peak ceiling"); GUI `states_and_lanes` `outcome=limited` |
| A mix that clipped before is saved with no clipped sample and no warning | 2, 3, 6, 10, 11, 12 | `mixer::sum_does_not_clip`; `measure_mix` unclipped; `ac4_..` (stems at +12 dB clip in playback, `clipped_samples == 0`, decoded peak < 1.0, no "clipped" in the text); GUI `states_and_lanes` `clipped=0` |
| Backing saved before → Normalize gives -14.0 ± 0.1, same length, stems unchanged, `track.json` records loudness and gain | 4, 5, 7, 8, 11, 12 | `backing_normalize_tests` tone; `repository_tests` replace; `ac5_..` (fixture 202: frames equal, stems byte-identical, `loudness.{lufs,true_peak,gain_db}` set, `mix`, tempo and loops unchanged); GUI `editor_normalize` step 2 |
| Already normalized → not rewritten, line says at the target | 3, 7, 9, 10, 11, 12 | `decide` 0.1 rule; `backing_normalize_tests` record shortcut and measured path; `ac6_..` (bytes and mtime unchanged, outcome `at-target`, text contains "Already at the target"); GUI step 4 |
| Cancel during Normalize → file byte-identical | 5, 7, 8, 10, 11, 12 | `backing_normalize_tests` cancel in the write pass; `editor_tests` cancel/shutdown; `ac7_..` (pace + cancel at progress ≥ 0.5; bytes, `track.json`, no part, no trash); GUI step 5 |
| No file yet → Normalize disabled | 8, 10, 11, 12 | `editor_tests` refusal "Save the backing first"; `editor-state.test.ts` `canNormalize`; `EditorPane.test.ts` disabled button; `ac8_..`; GUI step 6 |
| Track open in the Player → after Normalize it plays the normalized file | 8, 11, 12 | `player_tests` `release_for_audio`; `ac9_..` (Closed event, reopened samples bit-exact to the new file); GUI `editor_normalize_player_reopen` (tone power ratio = gain) |

## 8. Open questions

None that block the plan. The team made these decisions; the owner may overrule any of them:

1. **Save also releases an open Player session** of the track when it writes the file (§2.8). Before, the Player played
   the old PCM after a re-save. This follows requirement 7's intent and costs nothing.
2. **The running job's own button becomes Cancel** (Save → Cancel, Normalize → Cancel), and the separate Cancel button
   of the save progress is gone (§2.10). This is the same pattern as Autodetect BPM and Detect BPM. It is needed to fit
   the row in the 380 px pane at 1024x640.
3. **The Editor still plays the mix at the lane gains, not normalized.** The CLIP badge still lights for a hot mix
   while listening in the Editor, even though the saved file will not clip. Normalizing Editor playback would need the
   loudness of the whole mix, recomputed after each gain change, which is a separate feature.
4. **The old file of a Normalize goes to the repository trash**, like a re-saved backing. Each Normalize that changes
   the file adds one trash entry.
5. **Normalize keeps the file's bit depth** (16, or 24 when a stem was 24-bit), with no dither, as Save does. Scaling
   16-bit audio again adds at most half an LSB of rounding (about -101 dBFS).
6. **"Too quiet"**: a backing whose gated loudness is undefined (silence, below -70 LUFS, or shorter than 400 ms) is
   written as mixed (Save) or left alone (Normalize), and the line says so.
7. **Only stereo files are normalized.** Save always writes stereo; a hand-made mono backing is refused with a message
   instead of being turned into stereo.
