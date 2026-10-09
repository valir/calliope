# Plan: drop empty stems at import (increment to gui-stem-extraction)

Spec: `specs/gui-stem-extraction.md`, requirement 8 and the last acceptance criterion (commit
4c67b9c). Base: the built feature (`specs/gui-stem-extraction.plan.md`, `.report.md`). Only the
delta is planned here.

## 1. Summary

When an import receives its stems from the edge-AI server, the GUI import job measures each stem
and leaves out the silent ones, so the new track lists only the stems that have content (for
example bass, drums, guitar, vocals). The user sees which stems were dropped on the "Import
finished" page, and the log records each dropped stem with its peak level. Owner decision:
"empty" means quieter than -50 dBFS, not exactly all zeroes (Demucs leaks faint noise into the
stems of instruments that aren't there).

## 2. Design

### 2.1 The measure (owner threshold -50 dBFS)
- **Sample peak over the whole stem, all channels.** `peak = max |s| / 2^(bits-1)` over every
  sample of every channel, and `peak_dbfs = 20·log10(peak)`, which is `-inf` for all zeroes.
- A stem is **empty if and only if `peak_dbfs < SILENT_STEM_DBFS`**, with
  `pub const SILENT_STEM_DBFS: f64 = -50.0;` in `src/stem_audio.rs`. That constant is the only
  place the number appears. A stem exactly at -50.0 dBFS is kept.
- **Why peak and not RMS/LUFS:**
  - Dropping a stem can't be undone (only a re-import brings it back), so the measure must never
    drop real music. A whole-stem RMS or loudness figure falls a long way when an instrument
    plays only briefly. A 10-second piano intro in a 5-minute song would then look "empty".
    Peak keeps any stem in which anything audible happens, even once.
  - Peak is the direct generalisation of the spec's "all zeroes": with the threshold at `-inf` it
    is exactly that test.
  - The cost: a single leakage transient above -50 dBFS keeps a stem. That is harmless, because
    it is what happens today.
- **Early exit:** decoding stops at the first sample at or above the threshold. Audible stems
  therefore cost almost nothing to check. Silent stems are decoded in full, which is fast
  because they are mostly constant. `claxon` is already built with `opt-level = 3` in dev builds.
- **Decoding:** `claxon` (already a GUI dependency). No new dependencies, and no symphonia.
- **Interface** (in `src/stem_audio.rs`):
  ```rust
  pub const SILENT_STEM_DBFS: f64 = -50.0;
  /// Some(peak_dbfs) if every sample of every channel is below SILENT_STEM_DBFS (None inside
  /// means digital silence, -inf), None if the stem is audible (stops at the first loud sample).
  pub fn silent_peak(path: &Path) -> Result<Option<Option<f64>>, String>;
  ```
  The implementer may use a small enum instead (`Level::Audible | Level::Silent { peak_dbfs:
  Option<f64> }`), which reads better. It works for any channel count and for 4..=32 bits. It
  does not use `StemReader`'s 1-2 channel and 8-24 bit limits. The decision uses integers
  (`|s| as i64` against a per-bit-depth limit). The dB value is computed only for logging.

### 2.2 Where the check runs: the GUI import job
In `import_job::run_extract`, in the existing stem-receiving loop, each stem is checked right
after `fetch_stem` writes `stems/<name>.flac.part` in the staging folder:
- **Audible:** `finish_stem` runs, as today.
- **Silent:** a new `Staging::discard_stem_part(name)` removes that `.part` file. It removes only
  that file: the name is validated, it must be a regular file (not a symlink), and it must be
  inside this staging folder. The stem then goes into a `dropped` list.
- **Unreadable (claxon error):** the stem is **kept**. It gets a log line and no other change.
  We drop a stem only after proving it is silent, and this keeps today's behaviour for odd
  FLACs. The existing test "16 stems are fine" serves undecodable `fLaC…` bodies and must still
  pass.

After the loop, `build_meta` gets only the kept stems, in the server's order, and the track
commits as before. Staging plus one atomic rename is unchanged, so a crash or cancel during the
check leaves no visible track.

Why the GUI and not `calliope-stems` or `calliope-lib`:
- **API v1 doesn't change.** It already allows 1..=16 stems. The client checks only `≤ 16`,
  names and duplicates, and the GUI validation (`track_meta`, `MAX_STEMS`) only needs at least
  one stem. Nothing in the protocol or the conformance suite assumes 6 stems.
- **No redeployment on archserver.** The server and the separator stay as they are, and the
  check works with any separator.
- **No new dependency in the server.** The GUI already decodes FLAC.
- **The GUI knows what it dropped,** so it can tell the user.
- **Cost:** silent stems are still downloaded. Digital silence compresses to a few KB, near
  silence to a few MB, and this is on the LAN. `calliope-lib` gains nothing, because only the
  GUI uses the check.

### 2.3 All stems silent
If every stem is silent, the job **fails** and creates nothing:
- Stage `server`, with the message `Every stem is silent (below -50 dBFS), so no track was
  saved`, built from the constant.
- The existing failure path abandons staging and deletes the server job. It keeps the prepared
  `audio.flac` in `import-tmp`, so the user can Extract again or Discard. The user's source
  file is never touched.

Why: a stem track needs at least one stem (`validate_for_write`). Keeping every silent stem
would contradict requirement 8, and keeping "the loudest" would be an arbitrary track of
leakage. A silent input is almost certainly a mistake (a wrong file, or a silent video
section), and failing loses no data. This is the same UI flow as any server-stage failure: back
to the edit pane with the message.

### 2.4 What the user and the log see
- **Rust (`eprintln`, stderr/journal)**, one line per measured stem outcome:
  - `calliope: import job=<job> stem=<name> dropped peak_dbfs=<-59.9|-inf> threshold_dbfs=-50`
  - `calliope: import job=<job> stem=<name> kept level=unknown (<error>)` for undecodable stems
  - one summary line `calliope: import job=<job> stems kept=<a,b,..> dropped=<c,d|none>`

  The peak is printed with one decimal.
- **Event and snapshot:** `ImportEvent::Saved { track, dropped: Vec<DroppedStem> }` and
  `JobSnapshot.dropped: Vec<DroppedStem>`. The list is always serialised, empty when nothing was
  dropped, and it survives re-attach through `get_import_job`. The type is
  `DroppedStem { name: String, peak_dbfs: Option<f64> }`, where `None` means digital silence,
  because serde turns `-inf` into `null` anyway. TS: `interface DroppedStem { name: string;
  peak_dbfs: number | null }`, and `dropped: DroppedStem[]` on `JobSnapshot` and on the `saved`
  event.
- **UI:**
  - On the "Import finished" page, under `Saved <title> with <n> stems.`, a second line
    appears when the list is not empty: `Dropped silent stems: piano, other.` It uses the
    server's order, plain text and the normal text colour (it's not an error).
  - The UI log line becomes `calliope-ui: import saved id=<id> stems=<n> original=<b>
    dropped=<a,b|none>`. The field is appended at the end, so the existing `saved id=` waits
    still match.

### 2.5 Consumers of a track with fewer than 6 stems: no changes needed
- `track_meta` validation: needs 1..=16 stems with unique names (a 2-stem v2 track is already
  tested).
- Repository loading, the Library "S" badge (by `type`), and the tablature and `original.flac`
  name-clash checks (based on the listed stems) all work with any count.
- Editor lanes and mixes: built from `stems`; `library-editor` already has 2-, 4- and 6-stem
  tracks.
- Backing save: compares stem lists.
- **No migration:** existing tracks are never rewritten or re-measured. The check runs only
  inside a new import.
- **Keep original:** `original.flac` is the full mix and is independent of the stems. It is
  adopted as before whatever was dropped. In the all-silent failure it isn't adopted, or is
  moved back by `abandon`, as in any failure.

### 2.6 Test stand-ins
- New fixtures in `tests/fixtures/import/stems-quiet/`: 1 s, mono, 8 kHz, 16-bit FLAC files, made
  by `make-fixtures.sh`:
  - `silent.flac` (all zeroes)
  - `minus60.flac` (440 Hz sine, amplitude 0.001, about -60 dBFS)
  - `minus45.flac` (440 Hz sine, amplitude 0.005623, about -45 dBFS)
- New `stub-separator` modes:
  - `sparse`: copies the 6 stems, then overwrites `piano` with `silent.flac`, `other` with
    `minus60.flac` and `guitar` with `minus45.flac`. Expected result: bass, drums, guitar and
    vocals are kept, and piano and other are dropped.
  - `silent`: all 6 stems are `silent.flac`.

  The server and its conformance tests are untouched (they don't use these modes).
- Boundary unit tests write FLAC files in memory with `flacenc` (the existing `write_flac`
  helper in `stem_audio.rs` tests).

## 3. Tasks

All commands run in `src/calliope-gui/`. "Headless suite" means `npm test`, which must pass with
`DISPLAY` unset.

1. **Quiet stem fixtures**
   - files: `tests/fixtures/import/make-fixtures.sh`, `tests/fixtures/import/README.md`,
     new `tests/fixtures/import/stems-quiet/{silent,minus60,minus45}.flac`
   - does: after the "six 1 s mono 8 kHz stems" block, add exactly:
     ```bash
     # quiet stems for the empty-stem check: digital silence, about -60 dBFS, about -45 dBFS
     mkdir -p stems-quiet
     ff -f lavfi -i "anullsrc=r=8000:cl=mono" -t 1 -sample_fmt s16 -c:a flac stems-quiet/silent.flac
     ff -f lavfi -i "aevalsrc=0.001*sin(2*PI*440*t):s=8000:d=1" -ac 1 -sample_fmt s16 -c:a flac stems-quiet/minus60.flac
     ff -f lavfi -i "aevalsrc=0.005623*sin(2*PI*440*t):s=8000:d=1" -ac 1 -sample_fmt s16 -c:a flac stems-quiet/minus45.flac
     ```
     Run `bash tests/fixtures/import/make-fixtures.sh` (without FORCE). Add one README table
     row: `` `stems-quiet/{silent,minus60,minus45}.flac` `` | 1 s, mono 8 kHz, 16-bit: digital
     silence, a 440 Hz sine at about -60 dBFS, and one at about -45 dBFS (used by the stub
     separator's `sparse` and `silent` modes).
   - done when:
     - the 3 files exist
     - `git status --porcelain tests/fixtures` shows only the new files and the 2 edited text
       files
     - `ffmpeg -nostdin -i stems-quiet/minus60.flac -af volumedetect -f null - 2>&1 | grep max_volume`
       shows between -61 and -59 dB, minus45 shows between -46 and -44, and silent shows
       `-inf` or ≤ -90
   - routine: yes

2. **Stub separator modes `sparse` and `silent`**
   - files: `tests/support/stub-separator`
   - does: add `sparse | silent` to the accepted modes (the `ok | bad-output | not-flac) ;;`
     line), document them in the header comment, and in the final `case` add:
     ```bash
       sparse)
         cp -- "$STEMS"/*.flac "$out/"
         cp -- "$QUIET/silent.flac" "$out/piano.flac"
         cp -- "$QUIET/minus60.flac" "$out/other.flac"
         cp -- "$QUIET/minus45.flac" "$out/guitar.flac"
         ;;
       silent)
         for f in "$STEMS"/*.flac; do cp -- "$QUIET/silent.flac" "$out/$(basename -- "$f")"; done
         ;;
     ```
     Add `QUIET="$HERE/../fixtures/import/stems-quiet"` next to `STEMS=`.
   - done when:
     - `bash -n tests/support/stub-separator` succeeds
     - `STUB_SEPARATOR_MODE=sparse tests/support/stub-separator tests/fixtures/import/untagged.flac <tmpdir> m`
       writes 6 files, and `cmp` shows that piano equals `silent.flac` and guitar equals
       `minus45.flac`
     - the headless suite still passes
   - routine: yes

3. **The measure: `stem_audio::silent_peak` + `SILENT_STEM_DBFS`**
   - files: `src/stem_audio.rs`
   - does: implement §2.1 (constant, function or enum, integer decision, early exit, any
     channel count and 4..=32 bits, error text `Cannot decode stem <file>: …`). Unit tests,
     using the existing `write_flac` helper:
     - all-zero stereo → silent, `None` (−inf)
     - 16-bit: max |s| = 103 (−50.05 dBFS) → silent; 104 (−49.97) → audible
     - a negative peak −104 → audible, which proves `abs` is used, and `i32::MIN`-safe
     - 24-bit: 26526 → silent, 26527 → audible
     - a stereo file whose only loud sample is in the right channel, in the last block → audible
     - a 1-sample spike at −49.9 dBFS in 10 s of silence → audible
     - not-FLAC bytes → `Err`
     - the committed fixtures: `silent` → `None`, `minus60` silent with a peak in (−61, −59),
       `minus45` audible, every file in `stems/` audible
   - done when: `cargo test -p calliope-gui stem_audio` passes and the headless suite passes
   - routine: no

4. **`Staging::discard_stem_part`**
   - files: `src/repository.rs`, `src/repository_tests.rs`
   - does: `pub fn discard_stem_part(&self, name: &str) -> Result<(), String>`. It validates the
     name (via `stem_file`) and removes `stems/<name>.flac.part` only if it is a real file (not
     a symlink); anything else is an error and nothing is removed. Tests:
     - the part file is removed and the other stems stay
     - an invalid name → Err
     - a symlink part (pointing outside) → Err, and the target is untouched
     - a missing part → Err
     - `commit` with the reduced meta succeeds and leaves no `.part` in `stems/`
   - done when: `cargo test -p calliope-gui repository` passes and the headless suite passes
   - routine: no

5. **Import job: measure, drop, report**
   - files: `src/import_job.rs`, `src/import_job_tests.rs`, `src/ipc.rs` (only if snapshot
     serialisation needs it), `tests/acceptance_stem_extraction.rs` and any other `tests/*.rs`
     that compiles `src/import_job.rs` by `#[path]` (add `#[path = "../src/stem_audio.rs"] mod
     stem_audio;`; find them with `grep -rn 'src/import_job.rs' tests/`)
   - does: §2.2-2.4 in `run_extract`:
     - measure after each `fetch_stem`, then finish or discard
     - log lines
     - the all-silent failure (before `adopt_original`)
     - `build_meta` with the kept stems
     - `DroppedStem`, `JobSnapshot.dropped` (reset on a new extraction and on cancel)
     - `ImportEvent::Saved { track, dropped }`

     Check `cancel` between stems as today. New tests in `import_job_tests.rs` (real
     `calliope-stems` + stub):
     - `sparse` → Saved, `track.stems` names = vocals, drums, bass, guitar in the server's
       order; `stems/` holds exactly those 4 `.flac` files; `dropped` = piano (None), other
       (Some(p) with p in (−61, −59)); the snapshot has the same `dropped`; `import-tmp` job
       removed
     - `sparse` + keep original → `original.flac` present, still 4 stems
     - `silent` → Failed, stage Server, the message contains "Every stem is silent"; no
       `tracks/` entry, no staging folder left, `audio.flac` still in `import-tmp`, and the
       server job deleted
     - `ok` → 6 stems and `dropped` empty (the existing tests keep passing unchanged)
   - done when: `cargo test -p calliope-gui import_job`, `cargo test -p calliope-gui --test
     acceptance_stem_extraction` and the headless suite pass
   - routine: no

6. **Frontend: types, state, "Dropped silent stems" line**
   - files: `src/ui/lib/ipc.ts`, `src/ui/lib/import-state.svelte.ts`, `src/ui/views/ImportView.svelte`,
     `src/ui/views/ImportView.test.ts` (and `src/ui/lib/ipc-import.test.ts` if fixtures there need
     the new field)
   - does: §2.4:
     - the `DroppedStem` type, and `dropped` on `JobSnapshot` and the `saved` event
     - store it on the job, from both the event and the snapshot
     - log `dropped=<names|none>`
     - render `Dropped silent stems: piano, other.` only when non-empty

     vitest:
     - a saved event with 2 dropped stems shows the line and logs `dropped=piano,other`
     - with an empty list there is no line and the log says `dropped=none`
     - re-attach from a snapshot in phase `saved` shows the line
   - done when: `npm run check` and `npm run test:ui` pass, and the headless suite passes
   - routine: no

7. **GUI e2e: an import ends with 4 stems**
   - files: `tests/gui_import_e2e.rs`
   - does: a new `#[ignore]` test `import_drops_silent_stems`, built like
     `import_local_audio` but with `Rig::new(..., "sparse", false, ["import-audio {media}/tagged.ogg"])`:
     - Extract, then `saved()`
     - assert the UI line has `stems=4` and `dropped=piano,other`
     - `track.json` is `type: stem` with stem names {bass, drums, guitar, vocals}, and `stems/`
       holds exactly those 4 `.flac` files (don't use `check_stem_track`, which expects 6)
     - `check_disk`, `assert_no_children`
     - screenshot `import-done-dropped`

     View the screenshot and confirm the "Dropped silent stems: piano, other." line is readable
     and the layout isn't broken.
   - done when: `DISPLAY=:1 npm run test:gui` passes (all import tests green) and the
     screenshot has been reviewed
   - routine: no

## 4. Test strategy
- **Unit tests:**
  - the measure at the threshold boundary, for 16 and 24 bits, both signs, every channel, the
    last block, early exit and errors (task 3)
  - `discard_stem_part` safety (task 4)
  - frontend state and rendering (task 6)
- **Import job, headless:** the real `calliope-stems` binary on 127.0.0.1 with the stub separator
  in `sparse`, `silent` and `ok` modes (task 5). The acceptance suite still covers the
  undecodable-body cases, which now count as "kept".
- **End to end:** the GUI import on display `:1` in the loopback-only namespace with stub mode
  `sparse`, plus a screenshot (task 7).
- **No test** uses the real model, archserver, the LAN or the internet. The stand-ins are the
  stub separator and the quiet fixtures.
- **Commands** (in `src/calliope-gui/`): `npm test` runs the whole headless suite, and
  `DISPLAY=:1 npm run test:gui` runs the GUI tests.

## 5. Deployment
- Only the GUI changes. The laptop gets it through `npm run build:app`, as usual.
- `calliope-stems` and the separator adapter on archserver are unchanged, so there is nothing to
  redeploy.

## 6. Manual checks (owner)
1. On the laptop, import a real song that has no piano (a typical rock song) against
   archserver. "Import finished" says `Dropped silent stems: …` and lists piano and/or other.
   The track folder has only the listed stems.
2. Run `journalctl` or the terminal output with
   `grep "calliope: import job=.* dropped peak_dbfs"` to read the peaks of the dropped stems. If
   a stem you consider empty was kept (leakage peaks above -50 dBFS), report the song. The
   constant `SILENT_STEM_DBFS` can be tuned.
3. Import a song with a short, quiet piano part (an intro only). The piano stem is kept.
4. Open the 4-stem track in the Editor. 4 lanes are shown, Mix Play works, and Save makes
   `backings/backing.flac`.
5. With "Keep the original mix" on, import again. `original.flac` is present, alongside the
   reduced stem list.
6. Your existing stem tracks still show 6 stems, and their `track.json` is unchanged (`md5sum`
   before and after opening the Library).

## 7. Acceptance mapping

| Criterion | Tasks | Tests |
|---|---|---|
| Req 8 / "when operation is complete, check if any stem is empty and drop it" (empty = below -50 dBFS, owner decision) | 3, 4, 5 | `stem_audio` boundary tests; `import_job_tests` sparse/silent/ok; `repository_tests` discard |
| Tracks have only the kept stems (for example bass, drums, guitar, vocals) | 5, 7 | import-job `sparse` disk asserts; GUI `import_drops_silent_stems` |
| The user is told what was dropped (architect addition) | 6, 7 | vitest ImportView; GUI screenshot `import-done-dropped` |
| Existing criteria keep passing | 5 | the existing headless and GUI suites unchanged |

## 8. Open questions
None. Decided here and recorded in `docs/architecture.md`:
- peak measure, strictly below -50 dBFS
- the check runs in the GUI
- an all-silent result fails the import with a clear message
- undecodable stems are kept
- no migration of existing tracks

The spec's criterion text still says "all zeroes". The owner may want to update it to "below
-50 dBFS" for consistency.
