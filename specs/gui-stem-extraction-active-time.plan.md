# Plan: empty stems are judged by audible time (increment to gui-stem-extraction)

Spec: `specs/gui-stem-extraction.md`, requirement 9 and its three acceptance criteria (commit c969054).
Base: c969054. This branch sits on two unmerged increments: clear-empty
(`specs/gui-stem-extraction-clear-empty.{plan,report}.md`) and server-peaks
(`specs/gui-stem-extraction-server-peaks.{plan,report}.md`). Only the delta is planned here.

## 1. Summary

A stem is now **empty** when it is audible for less than 15 s in total, counted in 100 ms windows
whose RMS level (loudest channel) is above -40 dBFS. Its sample peak no longer matters. When a job
is done, `calliope-stems` measures each stem's audible time and reports it in a new optional
`stem_levels` field, which replaces `stem_peaks`. The app doesn't download the stems reported as
empty, and checks every stem it does download with the same shared code. With a server that
doesn't report audible time (old server, a server-peaks build, or `--no-stem-levels`), the app
downloads every stem and measures it locally. The UI says "empty" instead of "silent".

## 2. Design

### 2.1 The measure, exactly: `calliope_lib::flac_level`
This new lib module replaces `flac_peak`. The server and the app both use it, so they agree bit for
bit. It uses claxon (already a lib dependency). No new crate, no symphonia.

- **Windows.** A window is `window_frames = sample_rate / 10` frames (integer division: 44.1 kHz →
  4410, 48 kHz → 4800, 8 kHz → 800). It always counts as `WINDOW_MS = 100` ms. Windows are
  back to back from frame 0 and don't overlap. They are independent of FLAC block boundaries,
  because the sums carry over from one block to the next. **A partial window at the end is
  ignored** (it's at most 99 ms and only lowers the count). A stem shorter than one window has
  0 windows, so 0 ms audible.
- **Level of a window.** For each channel, `S_c = Σ s²` over the window's samples, as integers.
  Each square fits in `u64`; the sum is a `u128`, which covers every bit depth. The window's level
  is `max_c S_c`, the loudest channel. The channels are not summed.
- **Comparison, all in integers.** "RMS above T dBFS" means `S/n > FS²·10^(T/10)`, where FS is
  `2^(bits-1)` and n is `window_frames`. Since S is an integer, that is the same as
  `S > sum_limit(bits, n, T)`, where `sum_limit = floor(n · 4^(bits-1) · 10^(T/10))`. When T is a
  multiple of 10 (the shipped -40 is), `sum_limit` is computed exactly in `u128`:
  `n · 4^(bits-1) / 10^(-T/10)`, floored. Other levels use an f64 fallback, documented as not
  guaranteed bit-identical across machines. The limit is computed once per stem.
  - Worked example, 16-bit at 8 kHz: the limit is `floor(800·2^30/10^4) = 85 899 345`.
    A window of constant |s| = 328 gives `86 067 200`: audible (-39.99 dBFS).
    |s| = 327 gives `85 543 200`: not audible (-40.02 dBFS).
- **Audible time.** `audible_ms = audible_windows × 100`. This is an integer, so the 15 s boundary
  is exact: 149 windows (14.9 s) is empty, 150 windows (15.0 s) is kept.
- **Early stop.** With `stop_at_ms = Some(m)`, the scan returns as soon as `audible_ms ≥ m`, with
  `complete = false`. The app uses this, so an audible stem costs only the audio up to its 15th
  audible second. The server passes `None`: it doesn't know the 15 s, and it reports the full time
  (also useful in logs).
- The peak (max |s|) is tracked in the same pass, for display only.

```rust
// src/calliope-lib/src/flac_level.rs
pub const WINDOW_MS: u32 = 100;
/// The level a window must exceed (RMS, loudest channel) to count as audible. Shared by the
/// server's report and the app's own check; the app ignores reports made at another level.
pub const AUDIBLE_LEVEL_DBFS: i32 = -40;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Activity {
    pub bits: u32, pub sample_rate: u32, pub channels: u32,
    pub peak: u64,            // max |sample|; a lower bound when !complete
    pub windows: u64,         // complete windows scanned
    pub audible_windows: u64, // windows above the level
    pub complete: bool,       // false = stopped early at stop_at_ms
}
impl Activity { pub fn audible_ms(&self) -> u64; /* audible_windows * WINDOW_MS */ }
pub fn window_frames(sample_rate: u32) -> u64;              // sample_rate / 10
pub fn sum_limit(bits: u32, frames: u64, level_dbfs: i32) -> u128;
/// Errors name the file, as flac_peak did: "Cannot open stem <f>: ..", "Cannot decode stem <f>: ..",
/// "Stem <f> has an unsupported format" (bits outside 4..=32, 0 channels, sample rate < 10).
pub fn scan(path: &Path, level_dbfs: i32, stop_at_ms: Option<u64>) -> Result<Activity, String>;
pub fn to_dbfs(peak: u64, bits: u32) -> Option<f64>;      // moved unchanged from flac_peak
```

Why `AUDIBLE_LEVEL_DBFS` lives in the lib: the level is part of *what is measured*. The server
must count at some level, and passing it per job would add a request parameter for no gain, since
both binaries are built from this repository. The **decision** (15 s) stays in the app only. Every
report carries its `level_dbfs` and `window_ms`, and the app uses a report only if both equal its
own constants. So a server built from another version can never make the app decide on a
different measure: it just falls back to downloading.

**Cost.** The previous increment measured a full claxon decode of 6 stems × 10 min (stereo,
44.1 kHz) at 2.3 s on archserver. The sum of squares adds one multiply-add per sample.
- **Server:** about 1 s for the 6 stems of a 4-minute song (full scans), against minutes of GPU
  work. It runs outside the job lock and is cancellable between stems, as today.
- **App:** a downloaded empty stem needs a full scan (about 0.2 s for a 4-minute stem). An audible
  stem stops at 15 audible seconds, usually within its first 15-60 s of audio.

### 2.2 Wire format (API v1, additive): `stem_levels` replaces `stem_peaks`
`GET /v1/jobs/<id>`, when `done` and measured:

```json
{"job":"…","state":"done","progress":1.0,"stems":["vocals","drums","bass","guitar","piano","other"],"error":null,
 "stem_levels":[{"name":"vocals","audible_ms":200600,"level_dbfs":-40,"window_ms":100,"peak_dbfs":-1.23},
                {"name":"piano","audible_ms":0,"level_dbfs":-40,"window_ms":100,"peak_dbfs":null}, …]}
```

- **The data.** `audible_ms` (integer), `level_dbfs` (integer) and `window_ms` (integer).
  `peak_dbfs` is for humans only (curl, logs), and `null` means digital silence. The client
  ignores it, so the 15 s / -40 dBFS thresholds never cross the wire as decisions.
- **Same rules as `stem_peaks` had:**
  - The key is omitted (never `null`) when the server measured nothing: an old server, or
    `--no-stem-levels`.
  - A stem the server couldn't decode has **no entry**, and the job still succeeds.
  - The key is present only on `done` jobs.
- **Types** (`stems_api`):
  - `pub struct StemLevel { name: String, audible_ms: u64, level_dbfs: i32, window_ms: u32, peak_dbfs: Option<f64> }`
    (`peak_dbfs` has `#[serde(default)]`)
  - `JobStatus.stem_levels: Option<Vec<StemLevel>>` with
    `#[serde(default, skip_serializing_if = "Option::is_none")]`
- **Validation**, `stems_api::check_stem_levels(stems, levels) -> Result<(), String>`, requires:
  - every name is in `stems`, with no duplicates
  - `window_ms` is in 1..=1000
  - `level_dbfs` is in -150..=0
  - `audible_ms ≤ 86_400_000` and `audible_ms % window_ms == 0`
  - at most `MAX_STEMS` entries

  `StemsClient::status` checks it **only when the state is `done`**. On any other state it drops
  the field (sets it to `None`). This closes the server-peaks report's minor: garbage on a failed
  job no longer hides the job's own error. A failed check gives
  `ClientError::Invalid("bad stem level list")`.
- **Why replace rather than keep `stem_peaks`:** no released app reads it (the server-peaks branch
  is unmerged), and a peak can't decide emptiness any more. Keeping it would mean dead protocol.
  The new key has a new name, so the two kinds of server can't be confused:
  - A deployed server-peaks build sends `stem_peaks`. The new app ignores the key (serde ignores
    unknown fields), so it downloads and checks every stem locally, exactly as requirement 9 asks.
  - An old app (or the server-peaks app) talking to the new server ignores `stem_levels`.
  - The API version stays 1, and health is unchanged.
- Removed: `StemPeak`, `check_stem_peaks`, `JobStatus.stem_peaks`, `flac_peak`, `--no-stem-peaks`.
  The new switch `--no-stem-levels` does the same job (an off switch, and the "old server" stand-in
  in tests).

### 2.3 Server
`jobs::Manager::measure_stems` keeps its structure (after `validate_output`, outside the lock,
cancel/stop checked between stems, state stays `running`). It now calls
`flac_level::scan(out/<name>.flac, AUDIBLE_LEVEL_DBFS, None)` and builds a `StemLevel` per stem.
Logs (prefix `calliope-stems: `):
- per stem: `job id=<id> stem=<name> audible_ms=<n> windows=<w> level_dbfs=-40 peak_dbfs=<x.x|-inf>`
- per failed stem: `job id=<id> stem=<name> audible=unknown error="<msg>"`
- summary: `job id=<id> measured=<n>/<total> ms=<ms>`

`Config.stem_peaks` is renamed to `stem_levels`, and the flag to `--no-stem-levels`.

New diagnostic mode, `calliope-stems --measure FILE...`:
- It prints one line per file: `<file> audible_ms=<n> windows=<w> level_dbfs=-40 peak_dbfs=<x.x|-inf>`,
  or `<file> error=<msg>` for a file it can't measure.
- It exits 0 when every file was measured and 1 otherwise. It never starts a server and doesn't
  need `--separator`.
- It shows the owner the exact numbers the app will use, so the existing real tracks can be checked
  before anything is re-imported (§6). It also serves the future `stems-empty-detection` work.

### 2.4 App
`stem_audio.rs` replaces `SILENT_STEM_DBFS`, `Level`, `silent_peak` and `level_of_peak` with:

```rust
/// A stem audible (100 ms windows above flac_level::AUDIBLE_LEVEL_DBFS) for less than this is
/// empty and is dropped at import. The only place the number appears.
pub const EMPTY_STEM_MIN_AUDIBLE_MS: u64 = 15_000;
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StemCheck {
    /// audible_ms >= the minimum; `audible_ms` is a lower bound when the scan stopped early.
    Audible { audible_ms: u64 },
    /// audible_ms < the minimum (exact). `peak_dbfs` for logs only (None = digital silence; always
    /// None for a server report).
    Empty { audible_ms: u64, peak_dbfs: Option<f64> },
}
pub fn is_empty(audible_ms: u64) -> bool;                // audible_ms < EMPTY_STEM_MIN_AUDIBLE_MS
pub fn check_stem(path: &Path) -> Result<StemCheck, String>;  // scan(path, AUDIBLE_LEVEL_DBFS, Some(MIN))
/// The decision for a server report; None when it was measured at another level/window (unusable).
pub fn check_reported(level: &StemLevel) -> Option<StemCheck>;
```

`import_job::run_extract` keeps its loop (server order, cancel check, progress, staging rules).
- **The server reports the stem with our level/window:** the stem counts as server-measured.
  - If `is_empty`, the stem is **not fetched**. It goes into `dropped`, with the log line
    `calliope: import job=<j> stem=<n> dropped audible_s=<x.x> min_audible_s=15.0 level_dbfs=-40 source=server`.
  - Otherwise it is fetched and checked locally, as below.
- **The server reports the stem at another level/window:** the log says
  `stem=<n> server report not used level_dbfs=<l> window_ms=<w>`, and the stem is treated as not
  reported.
- **Fetched stems:** `check_stem(part)`.
  - `Empty`: discard the part, add the stem to `dropped`, and log
    `… dropped audible_s=<x.x> min_audible_s=15.0 level_dbfs=-40 peak_dbfs=<x.x|-inf> source=local`.
  - `Audible`: keep it, and log `… kept audible_s>=15.0 source=local` (`audible_s=<x.x>` when the
    scan was complete, which can't happen for an audible stem with early stop, but keep the format
    general).
  - `Err`: keep it, and log `kept level=unknown (<e>)` as today. Only proven emptiness drops a stem.
- **Local proof of emptiness wins** over a server "audible" report (unchanged trust model).
- Summary line: `calliope: import job=<j> stems kept=<a,b> dropped=<c|none> server_levels=<n>/<total>`.
- `DroppedStem` becomes `{ name: String, audible_ms: u64 }`. `peak_dbfs` is dropped: the peak no
  longer explains anything, and the logs carry it. `ImportEvent::Saved` and `JobSnapshot.dropped`
  keep their shape otherwise.
- **All stems empty:** stage `server`,
  `"Every stem is empty (less than 15 s above -40 dBFS), so no track was saved"` (built from the two
  constants). Nothing is created, and the prepared audio stays for a retry, as today.
- **Wording.** The finished page says **"Dropped empty stems: piano, other."** The UI log line
  `import saved … dropped=<a,b|none>` is unchanged.
- **Unchanged:** existing tracks are not re-measured, `original.flac` is unaffected, and undecodable
  stems are kept.

### 2.5 Test stand-ins and fixtures
The canned stems are 1 s long today. Under the new rule all of them would be empty, and every `ok`
import would fail. So:
- **`tests/fixtures/import/stems/*.flac` are regenerated at 16 s** (same tones 220..770 Hz, mono
  8 kHz, about 18 dBFS below full scale). That is 16.0 s audible each, kept. About 60 KB each.
  The committed `library-editor/` stems are copies and stay untouched.
- **New `tests/fixtures/import/stems-activity/`** (mono 8 kHz 16-bit, sine tones gated on exact
  100 ms windows, so ±1 sample of rounding can't change a count). The recipes were checked by the
  architect with ffmpeg and an independent measurement:

  | File | Length | Audible | Peak | What it stands for |
  |---|---|---|---|---|
  | `bursts.flac` | 30 s | 9 500 ms | -8.0 dBFS | an empty stem with separation artifacts: a 9 s tonal burst at -17 dBFS plus 5 single-sample clicks at -8 dBFS (each click lifts one window to about -37 dBFS) |
  | `phrases.flac` | 32 s | 16 000 ms | -20.0 dBFS | a real part with pauses: eight 2 s phrases with 2 s gaps (longest run 2 s) |
  | `audible-14900ms.flac` | 20 s | 14 900 ms | -20.0 dBFS | boundary, empty: three pieces of 5.0 + 5.0 + 4.9 s |
  | `audible-15000ms.flac` | 20 s | 15 000 ms | -20.0 dBFS | boundary, kept: 5.0 + 5.0 + 5.0 s |

  About 200 KB in total. `stems-quiet/` is unchanged (silent, -60, -45 dBFS, 1 s; all empty now).
- **Stub separator modes:**
  - `sparse` changes to: piano = `stems-quiet/silent.flac`, other = `stems-activity/bursts.flac`,
    guitar = `stems-activity/phrases.flac`. **The kept/dropped sets don't change** (kept: vocals,
    drums, bass, guitar; dropped: piano, other), so most existing expectations stay valid. But
    `other` now has a loud peak, and `guitar` is in pieces.
  - New mode `boundary`: vocals/drums/bass = `stems/`, guitar = `audible-15000ms`
    (kept), other = `audible-14900ms` (dropped), piano = `bursts` (dropped).
  - `silent` and `undecodable` are unchanged. With `undecodable`, piano is junk and kept, and other
    is `minus60` (1 s), so it's empty.
- Unit tests of `flac_level` write exact FLAC files at test time with `flacenc` (already a dev
  dependency of the lib), e.g. constant-|s| windows at 327/328.
- **Old servers:**
  - The real binary with `--no-stem-levels`.
  - The scripted fake in `tests/acceptance_silent_stems.rs` (no levels).
  - A scripted fake that sends a **server-peaks-era `stem_peaks`** key and no levels. Everything
    must be fetched.
- "Never downloaded" is still proven by the server's request log.
- No test opens an audio device, uses archserver, the LAN or the real model.

## 3. Tasks

Commands run in `src/calliope-gui/` unless stated. "Headless suite" = `npm test` with `DISPLAY`
unset. Each task leaves the headless suite green.

1. **Fixtures: 16 s canned stems and the activity stems**
   - files: `src/calliope-gui/tests/fixtures/import/make-fixtures.sh`,
     `src/calliope-gui/tests/fixtures/import/README.md`
   - does:
     - In `make-fixtures.sh`, in the "six 1 s mono 8 kHz stems" loop, change `duration=1` to
       `duration=16`. Change the comment to `# six 16 s mono 8 kHz stems, different tones (16 s
       audible each: kept by the 15 s empty-stem rule)`.
     - After the `stems-quiet` block, add exactly:
       ```bash
       # stems for the audible-time rule (100 ms windows above -40 dBFS, 15 s minimum); the tone
       # is gated on whole 100 ms windows so rounding by one sample cannot change a count
       mkdir -p stems-activity
       gen() { ff -f lavfi -i "aevalsrc='$1':s=8000:d=$2" -ac 1 -sample_fmt s16 -c:a flac "$3"; }
       # 9 s burst at -17 dBFS + five one-sample clicks at -8 dBFS: 9.5 s audible, empty
       gen "0.14*sin(2*PI*700*t)*between(floor(t*10),50,139)+0.4*(eq(floor(t*8000),160400)+eq(floor(t*8000),168400)+eq(floor(t*8000),176400)+eq(floor(t*8000),184400)+eq(floor(t*8000),192400))" 30 stems-activity/bursts.flac
       # eight 2 s phrases with 2 s pauses: 16.0 s audible in pieces, kept
       gen "0.1*sin(2*PI*500*t)*lt(mod(floor(t*10),40),20)" 32 stems-activity/phrases.flac
       # boundary: 5.0 + 5.0 + 4.9 s = 14.9 s (empty) and 5.0 + 5.0 + 5.0 s = 15.0 s (kept)
       gen "0.1*sin(2*PI*500*t)*(between(floor(t*10),10,59)+between(floor(t*10),70,119)+between(floor(t*10),130,178))" 20 stems-activity/audible-14900ms.flac
       gen "0.1*sin(2*PI*500*t)*(between(floor(t*10),10,59)+between(floor(t*10),70,119)+between(floor(t*10),130,179))" 20 stems-activity/audible-15000ms.flac
       ```
     - In the README:
       - Change the `stems/` row to "16 s, mono 8 kHz, different tones (used by the stub
         separator; 16 s audible each)".
       - Add a `stems-activity/` row listing the four files with their length, audible time and
         peak (the table in plan §2.5).
       - Note under the `stems-quiet` row that these are all empty under the 15 s rule.
       - Change "Total size about 300 KB" to "about 850 KB".
       - Add one sentence: regenerating `../library-editor/` with `FORCE=1` would copy the 16 s
         stems.
   - done when (the orchestrator runs these; they generate the committed files):
     `rm -f tests/fixtures/import/stems/*.flac && bash tests/fixtures/import/make-fixtures.sh` succeeds;
     `for f in tests/fixtures/import/stems/*.flac; do ffprobe -v error -show_entries format=duration -of csv=p=0 $f; done`
     prints `16.000000` six times; `ls tests/fixtures/import/stems-activity | wc -l` is 4;
     `git status --short tests/fixtures/import` shows only `stems/`, `stems-activity/`, the script and
     the README; the headless suite passes (the peak rule still keeps every 16 s stem)
   - routine: yes

2. **Stub separator mode `boundary`**
   - files: `src/calliope-gui/tests/support/stub-separator`, `src/calliope-gui/tests/support/README.md`
   - does:
     - In `stub-separator`, add `ACT="$HERE/../fixtures/import/stems-activity"` after the `QUIET=`
       line.
     - Add `| boundary` to the line `ok | bad-output | not-flac | sparse | silent | undecodable) ;;`
       before `)`.
     - In the header comment, after the `undecodable` line, add
       `#   boundary    vocals/drums/bass as ok; guitar 15.0 s audible, other 14.9 s, piano short loud bursts (9.5 s)`.
     - In the final `case`, before `*)`, add exactly:
       ```bash
         boundary)
           cp -- "$STEMS"/*.flac "$out/"
           cp -- "$ACT/audible-15000ms.flac" "$out/guitar.flac"
           cp -- "$ACT/audible-14900ms.flac" "$out/other.flac"
           cp -- "$ACT/bursts.flac" "$out/piano.flac"
           ;;
       ```
     - In `tests/support/README.md`, add `boundary` with the same one-line description wherever
       the modes are listed.
   - done when: `bash -n tests/support/stub-separator` succeeds;
     `d=$(mktemp -d) && STUB_SEPARATOR_MODE=boundary tests/support/stub-separator tests/fixtures/import/untagged.flac $d m && cmp $d/other.flac tests/fixtures/import/stems-activity/audible-14900ms.flac && cmp $d/piano.flac tests/fixtures/import/stems-activity/bursts.flac && ls $d | wc -l`
     prints 6; the headless suite passes
   - routine: yes

3. **`calliope_lib::flac_level`**
   - files: new `src/calliope-lib/src/flac_level.rs`, `src/calliope-lib/src/lib.rs` (`pub mod flac_level;`
     + doc line), `src/calliope-lib/Cargo.toml` (description: "the stem level/audible-time scan")
   - does: §2.1 exactly (`flac_peak` stays until task 9). Unit tests (flacenc, `tempfile`):
     - `window_frames(44100) == 4410`, `(48000) == 4800`, `(8000) == 800`
     - `sum_limit(16, 800, -40) == 85_899_345`, `sum_limit(16, 4410, -40) == 473_520_144`,
       `sum_limit(24, 4410, -40) == 4410 * (1u128 << 46) / 10_000`
     - constant |s| = 328 for 150 windows at 8 kHz: 15 000 ms; |s| = 327: 0 ms (also negative values)
     - stereo: left 327, right 328: every window audible; both 327: none (channels not summed)
     - 149 vs 150 loud windows separated by silence: 14 900 vs 15 000
     - the trailing partial window is ignored: 150 loud windows + 799 loud frames gives 15 000 ms
       and `windows == 150`; 799 loud frames alone give 0 ms and 0 windows
     - windows span FLAC blocks: one loud window starting at frame 4000 (block size 4096) counts
       exactly once
     - early stop: a 60 s fully loud stem with `Some(15_000)` gives `audible_ms() == 15_000`,
       `complete == false`, `windows == 150`; with `None`, 60 000 ms, `complete`
     - the peak is exact with `None` (single sample -9000 in silence: peak 9000, 0 ms audible)
     - 24-bit 44.1 kHz stereo: a constant window just above/below the level
     - not FLAC, or a missing file: `Err` naming the file
     - committed fixtures (`../calliope-gui/tests/fixtures/import/`): `stems-activity/bursts.flac`
       9 500 ms with peak ≥ 13 000; `phrases.flac` 16 000; `audible-14900ms.flac` 14 900;
       `audible-15000ms.flac` 15 000; every `stems/*.flac` 16 000; `stems-quiet/minus45.flac`,
       `minus60.flac` and `silent.flac` 0
   - done when: `cargo test -p calliope-lib flac_level` and the headless suite pass; clippy clean
   - routine: no

4. **Protocol: `StemLevel` (alongside `stem_peaks` for now)**
   - files: `src/calliope-lib/src/stems_api.rs`, `src/calliope-lib/src/stems_client.rs`,
     `src/calliope-stems/src/jobs.rs` (the `JobStatus` literal gets `stem_levels: None` for now)
   - does: §2.2: the type, the field, `check_stem_levels`, and the done-only check in `status()`
     (apply the done-only rule to `stem_peaks` too). Tests:
     - serde: absent key / `null` → None; round trip; `None` serialises without the key; an entry
       without `peak_dbfs` parses
     - a server-peaks-era status with `stem_peaks` but no `stem_levels` parses with
       `stem_levels == None`
     - `check_stem_levels` rejects: an unknown name, a duplicate, `window_ms` 0 and 1001,
       `level_dbfs` 1 and -151, `audible_ms` 150 with `window_ms` 100, and 86_400_100. It accepts
       an empty list and a subset.
     - client (scripted): a done status with a bad list → `Invalid("bad stem level list")`; a
       failed status carrying a bad list → `Ok` with the server's error and `stem_levels == None`
   - done when: `cargo test -p calliope-lib --features client` and the headless suite pass
   - routine: no

5. **Server measures audible time; `--no-stem-levels`**
   - files: `src/calliope-stems/src/jobs.rs`, `src/calliope-stems/src/config.rs`,
     `src/calliope-stems/src/cli.rs` (flag, `--help`, parser tests), `src/calliope-stems/tests/conformance.rs`,
     `src/calliope-stems/README.md`, plus a mechanical rename of `--no-stem-peaks` to
     `--no-stem-levels` in `src/calliope-stems/tests/acceptance_server_peaks.rs`,
     `src/calliope-gui/src/import_job_tests.rs` and `src/calliope-gui/tests/acceptance_server_peaks.rs`
   - does: in the stems-crate tests (`conformance.rs`, `acceptance_server_peaks.rs`), every test that
     runs the stub's `sparse` mode switches to `boundary`, with its peak expectations recomputed
     (tones of `stems/` peak about 4096, guitar/other about 3277, piano about 13107, all 16-bit),
     because task 7 changes `sparse`'s content. Then §2.3: `measure_stems` uses one
     `flac_level::scan(.., None)` per stem and fills **both**
     `stem_levels` and (from the same `Activity`'s `peak`/`bits`) the old `stem_peaks`, so the
     current app keeps working until task 7. The flag turns both off. README: document
     `stem_levels` (shape, meaning, "not measured = no entry", the level/window fields); the flag
     row. Conformance tests (use the `boundary` and `silent` modes, not `sparse`, whose content
     changes in task 7):
     - `boundary` done: 6 entries in stem order, all `level_dbfs` -40 and `window_ms` 100;
       vocals/drums/bass 16 000, guitar 15 000, other 14 900, piano 9 500 with a `peak_dbfs`
       above -9
     - `silent`: six entries with `audible_ms` 0 and `peak_dbfs` null
     - `undecodable`: no piano entry, and the log has `stem=piano audible=unknown`
     - `--no-stem-levels`: neither key, in any state
     - no `stem_levels` key while queued/running
     - the shared client sees the same values
   - done when: `cargo test -p calliope-stems`, `cargo clippy --workspace --all-targets -- -D warnings`
     and the headless suite pass
   - routine: no

6. **`calliope-stems --measure FILE...`**
   - files: `src/calliope-stems/src/cli.rs`, `src/calliope-stems/src/main.rs`, `src/calliope-stems/README.md`
   - does:
     - Add `Parsed::Measure(Vec<PathBuf>)`.
     - `parse` returns it when the first argument is `--measure`. All following arguments are
       file paths (at least one, else the error `--measure needs at least one file`). Nothing else
       is allowed with it, and `--separator` is not required.
     - In `--help`, add the line `  --measure FILE...          print each FLAC file's audible time
       (100 ms windows above -40 dBFS) and exit`.
     - `main`, for `Measure(files)`: for each file call
       `calliope_lib::flac_level::scan(f, AUDIBLE_LEVEL_DBFS, None)`.
       - On success, print to stdout
         `{f.display()} audible_ms={a.audible_ms()} windows={a.windows} level_dbfs={AUDIBLE_LEVEL_DBFS} peak_dbfs={x}`,
         where `x` is `to_dbfs(a.peak, a.bits)` formatted `{:.1}`, or `-inf`.
       - On error, print `{f.display()} error={e}`.
       - Exit 1 if any file failed, else 0.
     - Parser tests: `--measure a.flac b.flac` gives the two paths; `--measure` alone is an error;
       `--measure` with no `--separator` is fine.
     - README: one paragraph under the options table.
   - done when: `cargo test -p calliope-stems cli` passes;
     `cargo run -q -p calliope-stems -- --measure tests/fixtures/import/stems-activity/bursts.flac tests/fixtures/import/stems-activity/audible-15000ms.flac`
     prints two lines containing `audible_ms=9500` and `audible_ms=15000` and exits 0; the same with
     an extra `tests/fixtures/import/not-flac.flac` prints an `error=` line and exits 1; the
     headless suite passes
   - routine: yes

7. **The app judges by audible time**
   - files: `src/calliope-gui/src/stem_audio.rs`, `src/calliope-gui/src/import_job.rs`,
     `src/calliope-gui/src/import_job_tests.rs`, `src/calliope-gui/tests/support/stub-separator`,
     `src/calliope-gui/tests/support/README.md`, `src/calliope-gui/tests/acceptance_silent_stems.rs`,
     `src/calliope-gui/tests/acceptance_server_peaks.rs` → renamed (`git mv`) to
     `tests/acceptance_server_levels.rs`, and any other GUI Rust test that names the removed symbols
   - does:
     - **API and import job:** the §2.4 API (the old `Level`/`silent_peak`/`level_of_peak`/
       `SILENT_STEM_DBFS` are deleted) and the import-job behaviour, logs, `DroppedStem` and the
       failure message.
     - **Stub `sparse` mode:** replace its three lines with
       `cp -- "$QUIET/silent.flac" "$out/piano.flac"`,
       `cp -- "$ACT/bursts.flac" "$out/other.flac"` and
       `cp -- "$ACT/phrases.flac" "$out/guitar.flac"`, and update its header comment and the README
       line.
     - **`stem_audio` unit tests:**
       - `check_stem` on the four activity fixtures: bursts → `Empty { audible_ms: 9500, peak_dbfs: Some(> -9) }`,
         phrases → `Audible`, 14 900 → `Empty`, 15 000 → `Audible`; `stems-quiet/silent.flac`
         → `Empty { 0, None }`
       - `check_reported`: 14 900 → `Empty`, 15 000 → `Audible`, level -50 → `None`,
         window 50 → `None`
       - the not-FLAC error is passed through
     - **Port the earlier increments' tests** (keep file names except the rename; update the header
       doc comments):
       - **Delete** the tests that pin the superseded peak rule: the -50 dBFS constant, the
         8/16/24/32-bit peak boundaries, Demucs-style -51/-49 leakage, the server-vs-local peak
         boundary decision. Also **invert** "a short loud passage in a long quiet stem is kept":
         a 2 s loud passage in a 10-minute quiet stem is now **dropped**.
       - **Keep, updated:** the tests of behaviour that didn't change, now based on audible time
         and `DroppedStem { name, audible_ms }`:
         - sparse keeps vocals/drums/bass/guitar in server order
         - keep-original
         - all-empty failure with the new exact message, and the retry
         - undecodable kept
         - staging symlink/crash safety
         - existing tracks not re-measured
         - the long-stem timing test
         - cancel while measuring, and the server killed while measuring
       - In `acceptance_server_levels.rs`, the scripted fake serves `stem_levels` items
         `(name, body, Option<(audible_ms, level_dbfs, window_ms)>)`. Cases:
         - malformed lists are rejected before any download
         - a server that calls a loud stem empty: dropped unseen (by design)
         - a server that calls an empty stem audible: overruled locally
         - a partial report fetches the unreported stems
         - a report at `level_dbfs: -50` or `window_ms: 50`: fetched and checked locally, log
           `server report not used`
         - **a server-peaks-era fake** sending only `stem_peaks`: every stem fetched, same result
         - a null or empty list behaves like an old server
       - **New** import-job tests against the real server:
         - `boundary`: kept vocals/drums/bass/guitar; dropped `other` (14 900) and `piano` (9 500)
           in server order; the request log has no GET for other/piano
         - `boundary` with `--no-stem-levels`: the same track, all 6 fetched,
           `server_levels=0/6`
         - `sparse`: `other` (peak -8 dBFS) dropped, `guitar` (phrases) kept
         - `silent`: zero stem GETs and the new message
   - done when: `cargo test -p calliope-gui stem_audio`, `cargo test -p calliope-gui import_job`,
     `cargo test -p calliope-gui --test acceptance_silent_stems`, `--test acceptance_server_levels`,
     `--test acceptance_stem_extraction` and the headless suite pass; `grep -rn "silent_peak\|SILENT_STEM_DBFS\|level_of_peak" src tests`
     finds nothing
   - routine: no

8. **Frontend wording**
   - files: `src/calliope-gui/src/ui/views/ImportView.svelte`, `src/calliope-gui/src/ui/lib/ipc.ts`,
     `src/calliope-gui/src/ui/views/ImportView.test.ts`, `src/calliope-gui/src/ui/acceptance_silent_stems.test.ts`
     (and `src/ui/lib/ipc-import.test.ts` if it builds a `DroppedStem`)
   - does:
     - `ImportView.svelte`: `Dropped silent stems:` → `Dropped empty stems:`.
     - `ipc.ts`: replace the `DroppedStem` comment with
       `/** A stem the server returned but the import dropped as empty (audible < 15 s); \`audible_ms\` is its audible time. */`
       and the type with `export interface DroppedStem { name: string; audible_ms: number }`.
     - In the tests:
       - every `peak_dbfs: <v>` inside a dropped entry → `audible_ms: 9500`
       - every `Dropped silent stems` → `Dropped empty stems`
       - `'Every stem is silent (below -50 dBFS), so no track was saved'` →
         `'Every stem is empty (less than 15 s above -40 dBFS), so no track was saved'`
       - the describe/it titles' "silent" → "empty"
   - done when: `npm run check` and `npm run test:ui` pass; `grep -rn "silent stems\|peak_dbfs" src/ui`
     finds nothing; the headless suite passes
   - routine: yes

9. **Remove `stem_peaks` and `flac_peak`**
   - files:
     - `src/calliope-lib/src/{stems_api,stems_client,lib}.rs`
     - delete `src/calliope-lib/src/flac_peak.rs`
     - `src/calliope-lib/tests/acceptance_client_hostile.rs`
     - `src/calliope-stems/src/jobs.rs`
     - `src/calliope-stems/tests/conformance.rs`
     - `src/calliope-stems/tests/acceptance_server_peaks.rs` → `git mv` to `acceptance_server_levels.rs`
     - `src/calliope-stems/README.md`
     - `docs/licences.md` (claxon purpose: "FLAC decoding of stems; stem audible-time scan in
       calliope-lib (app and server)")
   - does:
     - Delete `StemPeak`, `check_stem_peaks`, `JobStatus.stem_peaks`, the server's peak entries and
       the README `stem_peaks` section.
     - Port the stems-crate acceptance file to `stem_levels`:
       - the key is absent while queued/running and on failed/cancelled jobs, and never `null`
       - the exact entry shape
       - all-silent gives 0 ms entries
       - an unmeasurable stem has no entry; an empty list is an array
       - the JSON parses with an old-shaped client
       - 24-bit stems measure correctly
       - the state stays running while measuring, and cancel then gives `cancelled`
       - SIGTERM during measuring exits promptly
       - the measuring time for a long song is logged and bounded
     - Hostile-client tests: the 15 malformed variants, re-expressed for levels.
     - Add a lib serde test: a status containing `stem_peaks` (server-peaks shape) still parses, and
       the key is ignored.
   - done when: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and the
     headless suite pass; `grep -rn "stem_peaks\|StemPeak\|flac_peak\|no-stem-peaks" src docs/licences.md`
     finds nothing (outside `node_modules`)
   - routine: no

10. **GUI e2e: empty stems by audible time**
    - files: `src/calliope-gui/tests/gui_import_e2e.rs`
    - does:
      - Rename `import_drops_silent_stems` to `import_drops_empty_stems`. Assertions unchanged:
        `dropped=piano,other`, 4 stems, piano/other never fetched.
      - Also assert that the app's stderr has
        `stem=other dropped audible_s=9.5 min_audible_s=15.0 level_dbfs=-40 source=server`.
      - Re-take the `import-done-dropped` screenshot and look at it: the finished page reads
        "Dropped empty stems: piano, other."
      - Add `import_boundary_stems` (mode `boundary`): saved with 4 stems, `dropped=other,piano`,
        screenshot `import-done-boundary`, other/piano never fetched.
    - done when: `DISPLAY=:1 npm run test:gui` passes (all suites), and both screenshots show the new
      wording
    - routine: no

## 4. Test strategy
- **Unit tests:**
  - the measure (task 3): integer limits, the 327/328 level boundary, 149/150 windows, loudest
    channel, partial window, block spanning, early stop
  - the app decision (task 7): `check_stem`, `check_reported` with level/window mismatch
  - the wire type, validation and compatibility (tasks 4, 9)
  - the CLI parser (tasks 5, 6)
- **Server end to end:** conformance and acceptance suites run the real binary on 127.0.0.1 with
  the stub separator (`boundary`, `silent`, `undecodable`, `--no-stem-levels`).
- **Import job:**
  - the real server + stub, with levels on and off
  - scripted fakes for malformed/mismatched reports and for server-peaks-era and pre-peaks servers
  - "never fetched" is proven by the server's request log
- **Frontend:** vitest for the wording and the type.
- **GUI:** the import e2e in the loopback namespace (`sparse`, `boundary`), with screenshots
  reviewed by the agent.
- **Spec criteria in tests:**
  - short loud artifacts: `bursts.flac`, peak -8 dBFS, 9.5 s → dropped
  - a part in pieces: `phrases.flac`, 16 s in 2 s pieces → kept
  - the boundary: 14.9 s dropped, 15.0 s kept, both locally and server-reported
  - server-reported empty stems: not downloaded, listed as dropped
- **Never used:** the real model, archserver, the LAN or a sound device.
- **Commands** (in `src/calliope-gui/`): `npm test` (whole headless suite, all crates);
  `DISPLAY=:1 npm run test:gui`.

## 5. Deployment
- **Laptop:** `npm run build:app`, as usual. The new app works with every server: pre-peaks,
  server-peaks or this one.
- **archserver (the owner, by hand; agents never deploy):**
  1. Check out this branch (or `main` once merged) in the repository on archserver.
  2. **Check the unit file** before restarting: if `~/.config/systemd/user/calliope-stems.service`
     has `--no-stem-peaks` in `ExecStart` (from the optional check of the last increment), remove it
     and run `systemctl --user daemon-reload`. The flag no longer exists, and an unknown flag stops
     the server from starting.
  3. From the repository top, run `bash script/calliope-stems-deploy.sh`. It builds the release
     binary, installs it to `~/.local/bin/`, restarts the user unit and shows the last journal
     lines, which should include `listening`. Jobs in progress are lost, as with any restart.
  4. `calliope-stems --help | grep -e --no-stem-levels -e --measure` shows both new options.

## 6. Manual checks (owner)
1. **The numbers on your existing tracks.** Before re-importing anything, on the laptop from the
   repository top, run:
   `cargo run -q --release -p calliope-stems -- --measure ~/.local/share/calliope/tracks/01a124a2-*/stems/*.flac`
   (2 Minutes to Midnight), then the same for `01a124b2-*` (Losfer Words). It only reads the files.
   - The real parts show about 200-362 s (`audible_ms=200600` or more), and the stems you consider
     empty show less than 15 000.
   - For 2 Minutes to Midnight, `other` should show about 9-11 s, although its `peak_dbfs` is high.
   - Note which stems are under 15 000: those are the ones the import will drop.
2. Deploy the server (§5). Then, in the app, re-import **2 Minutes to Midnight** from its URL
   (`https://www.youtube.com/watch?v=YCmUqAffWS8`). Check that:
   - The finished page says "Dropped empty stems: …", listing the stems from step 1.
   - `journalctl --user -u calliope-stems` on archserver shows one
     `stem=… audible_ms=… windows=… level_dbfs=-40 …` line per stem and `measured=6/6 ms=…`.
     Note the `ms`.
   - The app's output (`grep "calliope: import"`) shows `server_levels=6/6` and `dropped …
     source=server` for each dropped stem.
   - The journal has no `…/stems/<dropped>` request.
3. The same for **Losfer Words** (`https://www.youtube.com/watch?v=7mMqOmeyzPU`).
4. Open both new tracks in the Editor and play them. Every kept stem plays. Nothing musical is
   missing: the dropped stems were really bleed or artifacts. The old tracks
   `01a124a2-…`/`01a124b2-…` are not changed by this feature. Delete them from the Library (they go
   to `trash/`) if you want to keep only the new imports.
5. Optional fallback check: add `--no-stem-levels` to the unit's `ExecStart`, `daemon-reload`,
   restart, and import one song again. You should see `server_levels=0/6`, every stem downloaded,
   the same stems dropped with `source=local`. Remove the flag afterwards.

## 7. Acceptance mapping

| Criterion | Tasks | Tests |
|---|---|---|
| Req 9: empty = audible < 15 s in 100 ms windows above -40 dBFS, loudest channel | 3, 7 | `flac_level` unit (327/328, 149/150, stereo, partial window); `stem_audio` `check_stem`/`check_reported` |
| Stems audible < 15 s are dropped, even with short loud artifacts | 1, 3, 7, 10 | `bursts.flac` (peak -8 dBFS, 9.5 s) in `flac_level`, `stem_audio`, import_job `sparse`/`boundary`; GUI `import_drops_empty_stems`, `import_boundary_stems` |
| A real part with pauses adding up to ≥ 15 s is kept | 1, 3, 7 | `phrases.flac` (8 × 2 s) in `flac_level`, `stem_audio`, import_job `sparse` (guitar kept); `audible-15000ms` in three pieces in `boundary` |
| Server-reported empty stems are not downloaded and are listed as dropped | 4, 5, 7, 10 | conformance `boundary` values; import_job `boundary` request-log asserts; `acceptance_server_levels`; GUI e2e fetch counts |
| The server measures each stem's audible time when a job is done | 3, 5, 9 | conformance; stems `acceptance_server_levels` (shape, absent before done, cancel/SIGTERM while measuring) |
| A server without audible time: the app downloads and checks every stem | 4, 5, 7, 9 | `--no-stem-levels` import test; server-peaks-era fake; pre-peaks fake (`acceptance_silent_stems`); serde tests |
| Same decision whoever measures | 3, 4, 7 | shared `flac_level`; report used only at the app's level/window (mismatch test); 14 900/15 000 both ways |
| Earlier criteria (requirement 8, all-empty failure, undecodable kept, staging safety) | 7, 8 | ported `acceptance_silent_stems`, import_job tests, vitest wording |

## 8. Open questions
None blocking. Decided here and recorded in `docs/architecture.md`:
- the window is `sample_rate/10` frames, and a trailing partial window is ignored
- "above" is strict, with an integer sum-of-squares comparison
- the level and window are lib constants that each report echoes; the app ignores mismatched
  reports
- `stem_levels` replaces `stem_peaks`, and `--no-stem-levels` replaces `--no-stem-peaks`
- `DroppedStem` carries `audible_ms` instead of `peak_dbfs`
- new UI wording "Dropped empty stems"
- a `--measure` diagnostic mode on the server binary
