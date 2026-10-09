# Plan: the server reports stem peaks; silent stems are not downloaded (increment to gui-stem-extraction)

Spec: `specs/gui-stem-extraction.md`, requirement 9 and its acceptance criterion (commit 165aa6e).
Base: f62ecf9, on top of the clear-empty increment (`specs/gui-stem-extraction-clear-empty.plan.md`,
`.report.md`). Only the delta is planned here.

## 1. Summary

When a job is done, `calliope-stems` measures the sample peak of every stem and adds it to the job
status as a new, optional field. The app applies its own -50 dBFS rule to those numbers. It doesn't
download the stems the server reports as silent, and lists them as dropped, as before. It downloads and
checks every other stem itself. With a server that doesn't report peaks (old build, or the new
`--no-stem-peaks` switch), the app behaves exactly as it does today.

## 2. Design

### 2.1 One measure, shared: `calliope_lib::flac_peak`
The integer peak scan moves from the GUI's `stem_audio::silent_peak` into a new module in
`calliope-lib`. It is used by both the server and the app, so they cannot drift apart.
`calliope-lib` gains `claxon = "0.4"`. That is pure Rust, Apache-2.0, already in `Cargo.lock` and
already in `docs/licences.md`. The lib stays free of Tauri/GTK and HTTP servers, which
`tests/frontend.rs` checks. No symphonia.

```rust
// src/calliope-lib/src/flac_peak.rs
pub struct Scan { pub bits: u32, pub peak: u64, pub complete: bool }
/// max |sample| over every channel (any channel count, 4..=32 bits; |i32::MIN| = 2^31 fits).
/// With `stop_at_dbfs = Some(t)` it returns at the first |s| >= limit(bits, t) (complete = false).
/// Errors name the file: "Cannot decode stem <file>: ...", "Stem <file> has an unsupported format".
pub fn scan(path: &Path, stop_at_dbfs: Option<f64>) -> Result<Scan, String>;
/// Smallest |s| that is NOT below `dbfs`: ceil(2^(bits-1) * 10^(dbfs/20)) (16-bit, -50 -> 104).
pub fn limit(bits: u32, dbfs: f64) -> u64;
/// peak < limit(bits, dbfs): the one comparison every keep/drop decision uses.
pub fn is_below(peak: u64, bits: u32, dbfs: f64) -> bool;
/// 20*log10(peak / 2^(bits-1)); None for peak 0 (digital silence, -inf). Display only.
pub fn to_dbfs(peak: u64, bits: u32) -> Option<f64>;
```

The lib knows no threshold. `SILENT_STEM_DBFS = -50.0` stays in the GUI's `stem_audio.rs`, still the
only place the number appears. The GUI's `silent_peak(path) -> Result<Level, String>` keeps its
signature and behaviour, and becomes a thin wrapper:
`scan(path, Some(SILENT_STEM_DBFS))`. If the scan stopped early, the stem is `Audible`. Otherwise
`is_below` gives `Silent { peak_dbfs: to_dbfs(..) }`. New:
`stem_audio::level_of_peak(peak: u64, bits: u32) -> Level`, the same decision for a number reported
by the server. All existing `stem_audio` tests stay unchanged and must pass.

### 2.2 Wire format (API v1, additive)
`GET /v1/jobs/<id>` gets one optional field, `stem_peaks`. It is present only when the job is
`done` and the server measured. The key is omitted otherwise, never `null`:

```json
{"job":"…","state":"done","progress":1.0,"stems":["vocals","drums","bass","guitar","piano","other"],"error":null,
 "stem_peaks":[{"name":"vocals","peak":21450,"bits":16,"peak_dbfs":-3.68},
               {"name":"piano","peak":0,"bits":16,"peak_dbfs":null}, …]}
```

- **`peak` + `bits` are the data.** `peak` is the exact max |sample| over the whole stem, every
  channel, integer. `bits` is the stem's bits per sample. The app decides with
  `is_below(peak, bits, SILENT_STEM_DBFS)`, the same integer comparison its own scan uses, so the
  decision is identical whoever measured. No float crosses the wire for the decision.
- **`peak_dbfs` is for humans only** (logs, curl). It is a number, or `null` for digital silence
  (`peak` 0). The client ignores it.
- **Not measured ≠ silence.** Digital silence is `"peak": 0`. A stem the server couldn't measure has
  **no entry**. A missing `stem_peaks` key means the server measured nothing (an old server, or
  `--no-stem-peaks`).
- **Types** in `stems_api`:
  - `pub struct StemPeak { name: String, peak: u64, bits: u32, peak_dbfs: Option<f64> }`
  - `JobStatus.stem_peaks: Option<Vec<StemPeak>>` with
    `#[serde(default, skip_serializing_if = "Option::is_none")]`
- **Validation**, `stems_api::check_stem_peaks(stems: &[String], peaks: &[StemPeak]) -> Result<(), String>`.
  It requires that:
  - every name is in `stems`
  - there are no duplicates
  - `bits` is in 4..=32
  - `peak <= 2^(bits-1)`

  `StemsClient::status` calls it and maps a failure to `ClientError::Invalid("bad stem peak list")`,
  the same strictness as the stem list.
- **Compatibility:**
  - Old app + new server: the old `JobStatus` has no `deny_unknown_fields`, so serde ignores the new
    key. API version stays 1, and health is unchanged.
  - New app + old server: the key is absent, so `default` gives `None`.
  - A server started with `--no-stem-peaks` sends exactly the old JSON shape.

### 2.3 Server: measure after the separator, before `done`
In `jobs::Manager::supervise`, after `validate_output` returns `Ok(stems)`:
- The server checks under the lock that the job isn't cancelled and the server isn't stopping. It
  then **releases the lock** and measures each stem in order with
  `flac_peak::scan(out/<name>.flac, None)`, a full decode with no early exit (the server doesn't know
  the threshold).
- It re-checks cancel/stop between stems. If either is set, it stops measuring, and the existing
  cancelled path runs.
- A stem whose scan fails gets no entry and a log line. **The job still succeeds.**
- Then the lock is taken again and the job becomes `done` with `stem_peaks` set (`Some(vec)`, even
  if empty).
- During measuring the state stays `running` with the last progress value. The done log's
  `duration_s` includes the measuring.

Logs (prefix `calliope-stems: `):
- one line per stem: `job id=<id> stem=<name> peak=<p> bits=<b> peak_dbfs=<x.x|-inf>`
- for a failed scan: `job id=<id> stem=<name> peak=unknown error="<msg>"`
- one summary line: `job id=<id> measured=<n>/<total> ms=<ms>`

Cost: claxon decodes far faster than real time, so 6 stems of a 4-minute song take a few seconds,
against minutes of GPU separation. It runs on the single worker thread, so it delays only the next
queued job. Cancel waits at most one stem's decode, well inside `CANCEL_WAIT` (10 s).

New flag `--no-stem-peaks`:
- Fields: `Config.stem_peaks: bool` (default true).
- Effect: the server skips the measuring and never sends the key.
- Purpose: an operational off-switch, and the "old server" stand-in in tests.
- Documented in `--help` and the README table.

The server gains `claxon` through `calliope-lib`, so `THIRD-PARTY-NOTICES.txt` is regenerated
(`npm run notices:stems`), as `tests/notices.rs` requires.

### 2.4 App: skip silent stems, still measure what it downloads
In `import_job::run_extract`, after `wait_final` returns `done`, the job builds
`reported: HashMap<&str, (u64, u32)>` from `st.stem_peaks`. It logs
`calliope: import job=<j> server_peaks=<n>/<total>`, where 0 means an old server. Then, in the
existing per-stem loop, which keeps the server order, the cancel check and the progress events:
- **The server reports the stem and `level_of_peak` says Silent:** the stem is **not fetched**. Its
  `DroppedStem { name, peak_dbfs: to_dbfs(peak, bits) }` goes into `dropped`. Log:
  `calliope: import job=<j> stem=<n> dropped peak_dbfs=<x> threshold_dbfs=-50 source=server`
- **Anything else** (reported audible, or not reported): `fetch_stem`, then the local `silent_peak`,
  exactly as today. The existing log lines gain ` source=local` at the end.
  - Owner direction: the app keeps measuring what it downloads. Audible stems stop at the first loud
    sample, so this costs almost nothing.
  - The fallback path is the very same code.
  - If the local check finds a stem silent that the server called audible, the local proof wins
    (dropped, `source=local`). That can only happen with a server bug, and it is today's behaviour.
- **Unchanged:**
  - Undecodable downloaded stems are kept.
  - The all-silent failure keeps the same message, and is reached with zero downloads when the
    server reports everything silent.
  - The summary line, `DroppedStem`, `JobSnapshot.dropped`, `ImportEvent::Saved`, the frontend and
    the UI log line.
  - Cancel and the staging rules. Skipped stems never create a `.part`.
  - `stems_done` counts skipped stems too, so the progress still reaches total.

Data safety: an app drops a stem without seeing it only on the word of the configured server.
That is what requirement 9 asks for. Reports are validated (§2.2), and the decision is the app's own
integer rule. The server keeps the file until the app's `DELETE` or the retention sweep. Nothing
in the user's repository is touched.

### 2.5 Test stand-ins
- Server tests use the existing stub modes. `sparse` gives piano = 0, other ≈ 33 (−60 dBFS) and
  guitar ≈ 184 (−45 dBFS), all 16-bit. `silent` gives six zeros.
- New stub mode `undecodable`: the `sparse` stems, except that `piano.flac` is
  `fLaC` + junk. It passes the server's magic check, but no scan can decode it. Result: the server
  reports no piano entry, the app downloads piano and keeps it (unknown level), and `other` is
  skipped as silent.
- "Old server": the real `calliope-stems` with `--no-stem-peaks`. The existing scripted `fake_ai`
  in `tests/acceptance_silent_stems.rs` also sends no peaks, and its tests must keep passing
  unchanged.
- "Never fetched" proof: the server's stderr request log
  (`request method=GET path=/v1/jobs/<id>/stems/<name> status=200`), which `TestServer` in
  `src/import_job_tests.rs` already collects.

## 3. Tasks

Commands run in `src/calliope-gui/`. "Headless suite" = `npm test` with `DISPLAY` unset.

1. **Shared peak scan in calliope-lib**
   - files: `src/calliope-lib/Cargo.toml` (`claxon = "0.4"`; dev-dep `flacenc` with the same
     version/features as the GUI's), `src/calliope-lib/src/lib.rs` (`pub mod flac_peak;`, doc
     comment), new `src/calliope-lib/src/flac_peak.rs`, `src/calliope-stems/THIRD-PARTY-NOTICES.txt`
     (regenerated with `npm run notices:stems`), `docs/licences.md` (claxon row purpose: "FLAC
     decoding of stems; stem peak scan in calliope-lib (app and server)")
   - does: §2.1 API. Port the loop of the GUI's `silent_peak` (integer, all channels, early exit).
     Unit tests write FLAC with flacenc:
     - all-zero stereo → `peak 0, complete`
     - `limit(16, -50.0) == 104`, `limit(24, -50.0) == 26528`
     - 16-bit spike 103 → `is_below`, 104 and −104 → not
     - with `Some(-50.0)`, a loud sample stops early (`complete == false`), and without a stop it
       gives the exact max
     - 32-bit `i32::MIN` → peak 2^31; 8-bit works
     - the only loud sample in the right channel of the last block is found
     - not-FLAC → Err containing the file name
     - fixtures `stems-quiet/{silent,minus60,minus45}.flac` → 0, 30..=34, 175..=185 (path
       `../calliope-gui/tests/fixtures/import/`)
     - `to_dbfs(0, 16) == None`
   - done when: `cargo test -p calliope-lib flac_peak` and `cargo test -p calliope-stems --test notices`
     pass; the headless suite passes (incl. `frontend.rs` dependency and licence checks)
   - routine: no

2. **GUI uses the shared scan**
   - files: `src/calliope-gui/src/stem_audio.rs`
   - does: `silent_peak` becomes the wrapper of §2.1 (its own decode loop is deleted). Add
     `pub fn level_of_peak(peak: u64, bits: u32) -> Level`, plus a test: (103, 16) → Silent with a
     dB value near −50.05; (104, 16) → Audible; (0, 24) → `Silent { peak_dbfs: None }`. All
     existing tests stay unchanged.
   - done when: `cargo test -p calliope-gui stem_audio`, `cargo test -p calliope-gui --test
     acceptance_silent_stems` and the headless suite pass
   - routine: no

3. **Wire type, validation, client**
   - files: `src/calliope-lib/src/stems_api.rs`, `src/calliope-lib/src/stems_client.rs`,
     `src/calliope-lib/tests/acceptance_client_hostile.rs`,
     `src/calliope-stems/src/jobs.rs` (the `JobStatus` literal gets `stem_peaks: None` for now)
   - does: §2.2 (`StemPeak`, the optional field with `default` + `skip_serializing_if`,
     `check_stem_peaks`, the check in `status()`). Tests:
     - serde: absent key → None; explicit `null` → None; present → round trip; `None` serialises
       without the key
     - an `OldJobStatus` struct copied from the f62ecf9 definition parses a status that has
       `stem_peaks`
     - `check_stem_peaks` rejects an unknown name, a duplicate, bits 3 and 33, and peak 2^15+1 at
       16 bits, and accepts an empty list and a subset
     - hostile fake: a done status with a bad peak list → `Invalid`; a status without the key →
       `Ok` with `stem_peaks == None`
   - done when: `cargo test -p calliope-lib --features client` and the headless suite pass
   - routine: no

4. **Stub separator mode `undecodable`**
   - files: `src/calliope-gui/tests/support/stub-separator`
   - does: on the line `ok | bad-output | not-flac | sparse | silent) ;;` add `| undecodable` before `)`.
     In the header comment, after the `silent` line, add
     `#   undecodable the sparse stems, but piano is "fLaC" + junk (passes the server's check, cannot be decoded)`.
     In the final `case`, before `*)`, add exactly:
     ```bash
       undecodable)
         cp -- "$STEMS"/*.flac "$out/"
         cp -- "$QUIET/minus60.flac" "$out/other.flac"
         printf 'fLaC this is not a decodable stream\n' >"$out/piano.flac"
         ;;
     ```
   - done when: `bash -n tests/support/stub-separator` succeeds;
     `d=$(mktemp -d) && STUB_SEPARATOR_MODE=undecodable tests/support/stub-separator tests/fixtures/import/untagged.flac $d m && head -c4 $d/piano.flac`
     prints `fLaC`, `cmp $d/other.flac tests/fixtures/import/stems-quiet/minus60.flac` succeeds,
     and `ls $d | wc -l` is 6
   - routine: yes

5. **Server measures and reports**
   - files: `src/calliope-stems/src/jobs.rs`, `src/calliope-stems/src/config.rs`,
     `src/calliope-stems/src/cli.rs` (flag, `--help`, parser tests), `src/calliope-stems/src/main.rs`
     only if the config needs it, `src/calliope-stems/tests/conformance.rs`,
     `src/calliope-stems/README.md` (API section: the `stem_peaks` shape and meaning, "not measured" =
     no entry; the flag in the options table)
   - does: §2.3 (`Job.stem_peaks`, the measure step outside the lock with cancel/stop checks, the
     logs, `--no-stem-peaks`). Conformance tests (stub, 127.0.0.1):
     - `sparse`: raw JSON while queued/running has no `stem_peaks` key; when done, 6 entries in
       stem order with `bits` 16, piano `peak` 0 and `peak_dbfs` null, other 30..=34, guitar
       175..=185, and vocals not below the threshold
     - `undecodable`: done, no piano entry, 5 entries, and the stderr log has `stem=piano
       peak=unknown`
     - `ok` with `--no-stem-peaks`: done, no key, stems still served
     - the shared client sees the same peaks
     - deleting a job while it is running still gives `cancelled` (existing test unchanged)
   - done when: `cargo test -p calliope-stems` and the headless suite pass; `cargo clippy --workspace
     --all-targets -- -D warnings` is clean
   - routine: no

6. **Import job skips silent stems**
   - files: `src/calliope-gui/src/import_job.rs`, `src/calliope-gui/src/import_job_tests.rs`
     (`TestServer::start_with(mode, extra_args)`; `start(mode)` calls it with none)
   - does: §2.4. Tests against the real server:
     - `sparse` (peaks on): the saved track and `dropped` are equal to the existing `check_sparse`
       expectations. The server log has GET `/stems/<n>` for vocals, drums, bass and guitar only, and
       never for piano or other.
     - `sparse` + `--no-stem-peaks`: the same track and `dropped`, all 6 stems fetched.
     - `silent` (peaks on): fails with "Every stem is silent"; zero stem GETs; no staging folder left;
       `audio.flac` kept.
     - `undecodable`: saved with 5 stems (piano kept); `dropped` = other only; piano fetched, other
       not.
     - Cancel requested right after `done` leaves nothing (existing cancel test pattern).
     - All existing `import_job` tests and `tests/acceptance_silent_stems.rs` (old-server fake) pass
       unchanged.
   - done when: `cargo test -p calliope-gui import_job`, `cargo test -p calliope-gui --test
     acceptance_silent_stems`, `cargo test -p calliope-gui --test acceptance_stem_extraction` and the
     headless suite pass
   - routine: no

7. **GUI e2e: dropped stems are not downloaded**
   - files: `src/calliope-gui/tests/gui_import_e2e.rs`
   - does: in `import_drops_silent_stems`, also assert that `r.dirs.stems_log()` contains GET
     requests for `/stems/vocals` but none whose path ends in `/stems/piano` or `/stems/other`. The UI
     result is unchanged. Re-take the `import-done-dropped` screenshot and look at it: it should be
     identical in content.
   - done when: `DISPLAY=:1 npm run test:gui` passes (all suites)
   - routine: no

## 4. Test strategy
- **Unit:**
  - the scan and its integer boundaries (lib, task 1)
  - the wrapper and `level_of_peak` (GUI, task 2)
  - the wire type, compatibility and validation (task 3)
  - the CLI flag parsing (task 5)
- **Server end to end:** the conformance suite runs the real binary on 127.0.0.1 with the stub
  separator (`sparse`, `undecodable`, `ok`, `--no-stem-peaks`).
- **Import job:**
  - the real server + stub, with peaks on and off
  - "never fetched" is proven by the server's request log
  - the scripted old-server fake (`acceptance_silent_stems.rs`) for the fallback
- **GUI:** the existing e2e in the loopback namespace, plus the request-log assertion.
- **No test** uses the real model, archserver, the LAN or a sound device.
- **Commands** (in `src/calliope-gui/`): `npm test` (whole headless suite, all crates);
  `DISPLAY=:1 npm run test:gui`.

## 5. Deployment
- **Laptop:** `npm run build:app` as usual. The new app works with the old server, so the order of
  updating doesn't matter.
- **archserver (the owner, by hand; agents never deploy):**
  1. `git pull` and `cargo build --release -p calliope-stems`
  2. `install -m755 target/release/calliope-stems ~/.local/bin/`. The separator script is
     unchanged.
  3. `systemctl --user restart calliope-stems`. Jobs in progress are lost, as with any restart.
  4. `journalctl --user -u calliope-stems -n 5` shows `listening`.
  5. `calliope-stems --licenses | grep -i claxon` lists claxon.

## 6. Manual checks (owner)
1. On archserver, after the redeploy, import a song with no piano from the laptop. The journal shows
   one `stem=… peak=… bits=… peak_dbfs=…` line per stem and `measured=6/6 ms=…`. Note the `ms`, the
   cost of measuring.
2. The app's output (`grep "calliope: import"`) shows `server_peaks=6/6`, and for piano/other
   `dropped … source=server`. The journal has no `path=/v1/jobs/<id>/stems/piano` request.
   "Import finished" lists the dropped stems as before.
3. The track folder holds only the kept stems, and they open and play in the Editor.
4. Optional fallback check: add `--no-stem-peaks` to the unit's `ExecStart`, then
   `daemon-reload` and restart. Import again: `server_peaks=0/6`, every stem downloaded, the same
   stems dropped with `source=local`. Remove the flag afterwards.
5. An old app build, if you still have one, imports fine from the new server.

## 7. Acceptance mapping

| Criterion | Tasks | Tests |
|---|---|---|
| Req 9: the server reports each stem's peak when the job is done | 1, 3, 5 | `flac_peak` unit; conformance `sparse`/`undecodable` |
| Stems reported below -50 dBFS are not downloaded and are listed as dropped | 2, 6, 7 | import_job `sparse`/`silent` request-log asserts; GUI `import_drops_silent_stems` |
| A server without peaks: everything downloaded and checked as before | 3, 5, 6 | `--no-stem-peaks` import test; `acceptance_silent_stems` fake; serde absent-key tests |
| The same decision whoever measures (owner direction) | 1, 2 | shared `limit`/`is_below`; the boundary 103/104 in lib and GUI |
| Measuring failure doesn't fail the job | 5, 6 | conformance `undecodable`; import_job `undecodable` |
| Old app + new server | 3 | `OldJobStatus` parse test |
| Earlier criteria (requirement 8, all-silent failure) | 2, 6 | existing suites unchanged |

## 8. Open questions
None. Decided here, following the owner's direction, and recorded in `docs/architecture.md`:
- the server sends the integer peak and bit depth; the threshold stays in the app
- a stem that wasn't measured has no entry
- the app still checks what it downloads
- the server measures in full, sequentially, before `done`
- a `--no-stem-peaks` switch exists
