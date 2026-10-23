# Plan: Export a track's backing to the BOSS RC-5, MP3 or OGG

Spec: `specs/gui-export.md`. Base: `852b4d4` on branch `spec/gui-export`.

## 1. Summary

A new **Export** view (Alt+6; Settings moves to Alt+7) shows the shared read-only track tree on the
left and, on the right, the selected track's band, album and title, its count-in, and an "Export..."
group: "to BOSS RC-5" (with a memory-number box), "MP3 file" and "OGG file". Rust decodes the backing
track with the external `ffmpeg`, prepends the count-in (4 beats of silence before the original, with 6 ticks that end on the
first beat) and either writes the RC-5's `ROLAND/WAVE/0NN_1/0NN_1.WAV` itself (32-bit float, stereo,
44.1 kHz, exactly a `fmt ` and a `data` chunk), or pipes the PCM into a second `ffmpeg` that encodes
MP3 (libmp3lame VBR q2) or Ogg Vorbis (q6) with tags. Every exported file is exactly 4 beats of
silence + the original track; a track without a tempo map cannot be exported (the count-in needs it).
Every export runs as one cancellable job,
writes a part file in the destination folder and renames it at the end, so a cancel or a failure
leaves nothing behind; the folders are remembered in `settings.json`.

## 2. Design

### 2.1 What is reused, what is new

| Area | Reused | New / changed |
|------|--------|---------------|
| Tree | `TrackBrowser` (read-only, order, search, keys, Collapse/Expand all), `lib` selection | `ExportView.svelte`; the export target rule (2.9) |
| Track data | `Repository::load_record`, `TrackRecord.missing`, `player_source::sources` order (first backing variant, then `audio`) | `export_rules::export_source` (2.2) |
| Tempo | `TempoStore::load` through the app's `PlayerTempoStore` (flushes an open Player session first), `TempoMap` | `export_audio::count_in_plan` (2.3) |
| Click | `metronome::Click` (the Player's own click) | mixed into the export (2.3) |
| ffmpeg | `Tools` discovery, `calliope_lib::process::spawn_raw_stdout`, `file:` inputs, `-protocol_whitelist file,pipe` | `process::spawn_with_stdin` (piped stdin); decoder/encoder argv builders; encoder check (2.4) |
| WAV | the capture `WavWriter` in `audio_out.rs` (already exactly RIFF + `fmt ` 16 + `data`) | moved to `src/wav.rs`, shared (2.5) |
| Files | `fsutil::sync_dir`, `repository::{sanitize_component, cut_bytes, inside_root}` (made `pub(crate)`) | part file + rename + created-folder cleanup in the job (2.6) |
| Dialogs | `Picker::pick_folder`, `ScriptedPicker` (records start folders), `calliope: dialog` log lines | kinds `export-rc5`, `export-audio` (2.7) |
| Settings | `SettingsStore`, lenient load | `rc5_dir`, `export_dir`, `rc5_memory` (2.7) |
| Job | the import job's shape: start returns a snapshot, `Channel` events, `get_*`/`watch_*`, cancel, `RunEvent::Exit` shutdown; `backing_render::Throttle` | `src/export_job.rs` `ExportState` (2.6), commands in `src/export_ipc.rs` (2.8) |
| Navigation | `VIEWS`, NavBar, `viewForKey` | `export` view after Player, digits 1-7 (2.10) |

The export never reads stems, the kept original or tablatures (requirement 3): `export_source` only
ever returns the first backing variant's file or `audio`.

### 2.2 Which file is exported (requirements 2, 3)

`src/export_rules.rs` (pure):

```rust
pub enum SourceKind { Mix, Audio }
pub struct ExportSource { pub kind: SourceKind, /// relative to the track folder
                          pub file: String }
/// The backing track of `t`, or the reason why there is none (shown in the pane).
pub fn export_source(t: &TrackRecord) -> Result<ExportSource, String>
```

| Track | Result |
|-------|--------|
| stem, `backings` non-empty, first file present | `Mix`, `backings[0].file` |
| stem, `backings` empty | `Err("No mix saved yet: save one in the Editor")` |
| stem, first backing file in `missing` | `Err("The saved mix (backings/backing.flac) is missing: save it again in the Editor")` |
| backing, `audio` present | `Audio`, `audio` |
| backing, `audio` in `missing` | `Err("The audio file (gone.flac) is missing")` |
| backing, `audio` null | `Err("This track has no audio file")` |

Only the default (first) variant is used; choosing among variants is out of scope. The path is
`Repository::tracks_dir().join(id).join(file)`, where `load_record` already validated the id and
the file names (strictly inside `<root>/tracks/<id>/`).

### 2.3 The count-in (requirement 12; owner answers 1-5, 2026-10-23)

Six ticks at beat offsets **0, 1, 2, 2.5, 3, 3.5** (durations 1/4, 1/4, then 1/8 four times), ending
exactly on the track's first beat, at the tempo of the first section.

- **The first beat** is `tempo.segments[0].start_ms` and the BPM is `segments[0].bpm`. Both are read
  from the tempo map **at export time** (nothing cached: `export_info` reads it for the pane,
  `prepare_export` for the plan, and `start_export` reads it again and recomputes the count-in), through
  `TempoStore::load(id, dir)` (the app's `PlayerTempoStore`, so an open Player session is flushed first).
  A later Editor feature will place the first beat precisely; the export just follows the map.
- **The Player's speed does not apply** and `beats_per_bar` is ignored: always 4 beats at the first
  section's BPM at 100 % (owner answer 5).
- **No tempo map = no export** (owner answer 2): the three buttons are disabled and the pane says
  "Set a tempo in the Player first: the export starts with a count-in". Rust refuses too
  (`prepare_export`, and `start_export` if the map was emptied meanwhile), reason `no-tempo`.
- **Layout, on the song's beat grid** (owner answer 3), all in whole frames at the export's output
  rate `R`, the beat rounded **once** as the Player's `count_in_plan` does:
  - `beat = round(R · 60 / bpm)` frames; `lead = 4 · beat` frames;
  - `first = round(start_ms · R / 1000)` frames (the first beat in the original);
  - output = `lead` frames of digital silence, then the original **unchanged** from its 0:00 (any intro
    or noise before the first beat is kept as is), so the output is exactly `lead + source_frames`
    frames long (owner answer 1: "the original length + the count-in duration");
  - tick *k* starts at output frame `first + round(offset_k · beat)` (`f64::round`, half away from
    zero, so the half-beat ticks are `first + 2·beat + round(beat/2)` etc.), which is output time
    `start_ms + offset_k` beats, i.e. `lead + first − 4·beat + offset_k·beat`; the count ends on the
    first beat at output frame `lead + first`;
  - the ticks are mixed over whatever is there: the prepended silence, or the intro when it is longer
    than about half a beat. A tick at or after the end of the output is dropped (only for a map whose
    first beat lies after the end of the audio).
- **The tick sound** (owner answer 4): the Player's click (`metronome::Click::new(R)`: 1500 Hz, 30 ms
  decaying sine, peak 0.5 = −6 dBFS), no accent. It is **added**; only samples under a tick are touched,
  and those are clamped to [−1, 1] (counted as `clipped` in the log and the done event). All other
  samples pass through bit-exactly.

```rust
// src/export_audio.rs
pub const TICK_BEATS: [f64; 6] = [0.0, 1.0, 2.0, 2.5, 3.0, 3.5];
pub struct CountInPlan { pub bpm: f64, pub beat_frames: u64, pub lead_frames: u64,
                         pub first_beat_frame: u64 /* in the output: lead + first */, pub ticks: [u64; 6] }
pub fn count_in_plan(map: &TempoMap, rate: u32) -> Option<CountInPlan>   // None for an empty map
/// Adds the ticks that overlap `block` (interleaved, `channels`, starting at output frame `start`).
/// Returns the samples clamped.
pub fn mix_ticks(block: &mut [f32], channels: usize, start: u64, plan: &CountInPlan, click: &[f32]) -> u32
```

Worked examples (fixtures 406 and 407, 2.12), 44.1 kHz:
- 120 BPM, first beat at 500 ms: `beat = 22050`, `lead = 88200` (2.0 s), `first = 22050`; ticks at
  22050, 44100, 66150, 77175, 88200, 99225; the first beat is at output frame 110250; a 4 s original
  gives 88200 + 176400 = 264600 frames.
- 100 BPM, first beat at 3000 ms: `beat = 26460`, `lead = 105840` (2.4 s), `first = 132300`; ticks at
  132300, 158760, 185220, 198450, 211680, 224910 (all over the original's silent intro, which runs in
  the output from 105840 to 238140); the first beat is at output frame 238140.

So "the same length / duration" in the acceptance criteria means **the original's length + 4 beats**
(owner answer 1); "the same sound" means the original after the `lead` frames, plus the ticks.

### 2.4 Decoding and encoding (requirements 4.3, 5.2, 5.3)

One pipeline for all three targets, run on the job thread (`export_audio::run`):

```
ffmpeg (decoder) --f32le, R Hz, C ch--> Rust: lead + ticks -+--> wav::WavWriter (part file)       [RC-5]
                                                           +--> ffmpeg (encoder) stdin -> part   [MP3/OGG]
```

- **Format of the source**: `ffprobe -v error -protocol_whitelist file,pipe -select_streams a:0
  -show_entries stream=sample_rate,channels,duration:format=duration -of json file:<abs>`, parsed by
  the pure `export_rules::parse_stream_info(json) -> Option<StreamInfo { rate, channels, duration_s }>`
  (a stream duration wins over the format's). It runs in `prepare` (2.6).
- **Output format** (`export_rules::output_format(target, &StreamInfo) -> (rate, channels)`):
  RC-5 → (44100, 2) always (resampled by ffmpeg's default `swr`; a mono source is duplicated to
  both channels by `-ac 2`). MP3 → the source rate if it is one of libmp3lame's
  {8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000}, else 44100 when the rate is a
  multiple of 11025 (88.2/176.4 kHz) and 48000 otherwise (open question 9). OGG → the source rate.
  MP3/OGG channels: 1 for a mono source, else 2 (a source with more than two channels is downmixed).
- **Decoder argv** (`export_audio::decoder_args(src, rate, channels)`):
  `-nostdin -hide_banner -loglevel error -protocol_whitelist file,pipe -i file:<abs> -map 0:a:0
  -vn -sn -dn -f f32le (no length cap: the whole source is decoded) -acodec pcm_f32le -ac <C> -ar <R> pipe:1`, run with
  `process::spawn_raw_stdout`. Chunks go through a bounded `sync_channel(8)` to the job thread
  (backpressure), which keeps a partial sample across chunk boundaries.
- **Encoder argv** (`export_audio::encoder_args(kind, rate, channels, &Tags, part)`):
  `-nostdin -hide_banner -loglevel error -f f32le -ar <R> -ac <C> -i pipe:0 -map_metadata -1
  -fflags +bitexact -flags:a +bitexact` then for MP3 `-c:a libmp3lame -q:a 2 -id3v2_version 3
  -write_id3v1 0 -f mp3`, for OGG `-c:a libvorbis -q:a 6 -f ogg`, then the tags as
  `-metadata title=<t>`, `artist=<band>`, `album=<album>`, `track=<n>`, `date=<year>`, then
  `-y file:<part>`. `-y` is needed because Calliope creates the part file itself first
  (`create_new`, proving the file is ours); every other ffmpeg use keeps `-n`. The encoder writes to
  the **file**, not a pipe, because libmp3lame's VBR header (Xing/LAME tag with the encoder delay
  and padding) is written by seeking back; without it players show wrong VBR durations. Checked on
  ffmpeg 9.0.2: f32le through stdin into both encoders decodes back to exactly the input length.
- **Tags** (`export_rules::tags(&TrackRecord) -> Vec<(&'static str, String)>`): `title` always,
  `artist` = band and `album` when not empty, `track` = `position` only when the track has an album
  (tracks of "(no album)" ignore `position`, as in the tree) and `date` = `year` when known. Values
  are trimmed and control characters dropped. Through ffmpeg these become ID3v2.3 TIT2/TPE1/TALB/
  TRCK/TYER and Vorbis comments TITLE/ARTIST/ALBUM/TRACKNUMBER/DATE (ffmpeg maps `track`).
- **The stdin variant** of the process runner: `calliope_lib::process::spawn_with_stdin(program,
  args, cwd, on_stdout_line, on_stderr_line) -> io::Result<(Running, ChildStdin)>`, the same
  process-group, PDEATHSIG and cancel rules as `spawn`; `spawn_with` gets a `stdin: Stdio` parameter
  internally. Writing after the child died returns `BrokenPipe` (Rust ignores SIGPIPE), which the
  pipeline turns into the encoder's own error (its last stderr line) or "cancelled".
- **Missing tools / encoders**: no `ffmpeg` or `ffprobe` on `PATH` (re-discovered at each prepare,
  like the import's `with_rediscovery`) → "Install ffmpeg to export (ffmpeg and ffprobe are
  needed)". For MP3/OGG, `prepare` runs `ffmpeg -hide_banner -encoders` and the pure
  `export_rules::has_encoder(listing, "libmp3lame" | "libvorbis")` (a line whose second
  whitespace-separated field is the name); missing → "Your ffmpeg has no MP3 encoder (libmp3lame):
  install an ffmpeg build that includes it" (the same with "Ogg Vorbis encoder (libvorbis)"). Both
  checks run **before** the folder dialog, so the user is never asked for a folder for nothing.
- **Cancel**: the job loop waits on the chunk channel with a 50 ms timeout and checks the cancel
  flag; on cancel it calls `cancel()` on both children's handles, drops the encoder's stdin and waits
  for both (they are process-group leaders, so the 2 s TERM→KILL rule applies).
- **Pacing knob** (tests only): `ExportState::with_pace(f)` makes the loop sleep so that the export
  takes `f` × the audio's duration (`f = 1` = real time). The app sets it only in `e2e-hooks` builds
  from `CALLIOPE_E2E_EXPORT_PACE`, so the GUI cancel test has time to press Cancel.

### 2.5 The RC-5 WAV (requirements 4.2, 4.3, 4.5)

`ffmpeg`'s WAV muxer writes an 18-byte `fmt ` chunk plus a `fact` chunk for float data (checked on
ffmpeg 9.0.2 with `-bitexact -map_metadata -1`), and a `LIST` chunk without `-bitexact`. To be
sure the file has **no tags or extra chunks**, Calliope writes it itself. The capture writer in
`audio_out.rs` already does exactly this, so it moves to `src/wav.rs`:

```rust
/// Float32 stereo WAV: "RIFF" <36+n> "WAVE" "fmt " 16 {format 3 (IEEE float), 2 ch, rate,
/// rate*8 B/s, align 8, 32 bit} "data" <n>; 44-byte header, sizes patched on `finish`.
pub(crate) struct WavWriter { .. }
impl WavWriter {
    pub(crate) fn create(path: &Path, rate: u32) -> io::Result<Self>       // File::create (capture)
    pub(crate) fn from_file(file: File, rate: u32) -> io::Result<Self>     // the export's part file
    pub(crate) fn write(&mut self, samples: &[f32]) -> io::Result<()>
    pub(crate) fn finish(self) -> io::Result<()>                           // patch sizes + fsync
}
```

The RC-5 accepts stereo 44.1 kHz PCM WAV at 16/24-bit or 32-bit float, 0.35 s to 101 min
(Roland's RC-5 support article); Calliope's tracks are at most 15 minutes. Whether the RC-5 is happy
without a `fact` chunk is verified on the real unit (manual check 1, open question 7).

Path: `export_rules::rc5_target(base, n) = base/ROLAND/WAVE/{n:03}_1/{n:03}_1.WAV`. Checks, in this
order, each refusing with a message and writing nothing (requirement 4.4):

1. memory text (before any dialog): `export_rules::parse_memory(text) -> Result<u8, String>`:
   trimmed, 1 to 3 ASCII digits, value 1..=99; else "Enter a memory number from 1 to 99" (covers 0,
   100, "abc", "", "7.5", "-3");
2. after the dialog: the base is an existing folder ("<base> is not an existing folder");
3. `<base>/ROLAND` is a folder ("No ROLAND folder in <base>: is the RC-5 mounted?");
4. the base is not inside the repository root ("Cannot export into the repository folder");
5. `ROLAND/WAVE` and `0NN_1` are folders when they exist ("<path> is not a folder"), and the target
   is not a folder ("<path> is a folder").

`WAVE` and `0NN_1` are created only when the export starts (2.6). Only the one WAV is written;
other files in the memory's folder are never touched.

### 2.6 The export job (requirements 6-9)

`src/export_job.rs`, pure (no Tauri), one export at a time:

```rust
// ExportKind { Rc5, Mp3, Ogg } (serde "rc5" | "mp3" | "ogg") is declared in export_rules and re-exported here
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TargetReq { Rc5 { memory: String }, Mp3, Ogg }
pub struct ExportPlan { pub token: String, pub path: String, pub exists: bool }
pub struct ExportSnapshot { pub job: String, pub id: String, pub title: String, pub target: ExportKind,
    pub path: String, pub phase: ExportPhase, pub progress: f32, pub message: Option<String>, pub clipped: u32 }
pub enum ExportPhase { Writing, Finishing, Done, Failed, Cancelled }   // lowercase
#[serde(tag = "phase", rename_all = "lowercase")]
pub enum ExportEvent { Writing { progress: f32 }, Finishing, Done { path: String, clipped: u32 },
    Failed { message: String }, Cancelled }

pub struct ExportState { .. }
impl ExportState {
    pub fn new(tools: Box<dyn Fn() -> Tools + Send + Sync>) -> Self;
    pub fn with_pace(self, factor: f64) -> Self;
    /// Checks the chosen folder and makes a single-use plan (replacing an older one).
    pub fn plan(&self, p: PlanInput) -> Result<ExportPlan, String>;
    /// Starts the planned export. `overwrite` must be true when the target exists.
    pub fn start(&self, token: &str, overwrite: bool, tempo: &TempoMap, sink: Sink, on_done: Box<dyn FnOnce() + Send>)
        -> Result<ExportSnapshot, String>;
    pub fn cancel(&self, job: &str) -> Option<ExportSnapshot>;
    pub fn snapshot(&self) -> Option<ExportSnapshot>;
    pub fn watch(&self, sink: Sink) -> Option<ExportSnapshot>;
    pub fn shutdown(&self, cap: Duration) -> bool;     // RunEvent::Exit: cancel, wait, part removed
}
pub struct PlanInput<'a> { pub track: &'a TrackRecord, pub source: PathBuf, pub kind: ExportKind,
    pub memory: Option<u8>, pub chosen: &'a Path, pub repo_root: &'a Path, pub tempo: &'a TempoMap }
```

- **`plan`**: refuses while a job runs ("An export is running") and when the map is empty (2.3);
  runs ffprobe (2.4); checks the
  destination (2.5 for the RC-5; for MP3/OGG: the folder exists, is not inside the repository root,
  and the target is not a folder); computes the target path, `exists`, the output format, the
  count-in plan and the expected frames `lead + round(duration_s · R)`. MP3/OGG name:
  `export_rules::file_name(band, title, "mp3"|"ogg")` = `sanitize_component(band) + " - " +
  sanitize_component(title) + ".<ext>"` (just the title when the band is empty; the stem cut to 240
  bytes on a UTF-8 boundary; the track id when nothing is left), so `/ \ < > : " | ? *` and control
  characters become `_` like the Library's folder export (requirement 5.1).
- **`start`**: takes the plan (unknown or used token → "This export is no longer ready; press the
  button again") and the tempo map read again by the caller: recomputes the count-in (an empty map →
  refused, reason `no-tempo`). Re-checks the destination (the drive may have been unmounted) and the overwrite
  rule: target exists and `!overwrite` → refused "<path> already exists". Creates the missing `WAVE`
  and `0NN_1` folders (remembering which ones it created), creates the part file
  `<target dir>/.<target name>.calliope-<pid>-<n>.part` with `create_new`, then spawns the thread
  and returns the snapshot (phase `writing`, progress 0). A failure here (e.g. a read-only folder:
  "Cannot write to <dir>: Permission denied (os error 13)") removes what it created and returns
  `Err`, logged as `export failed`.
- **The thread**: `export_audio::run` → events `writing {progress}` (≤ 10/s via `Throttle`,
  progress capped at 0.99) → `finishing` (WAV: patch the header + fsync; MP3/OGG: wait for the
  encoder, check exit 0, open + fsync the part) → **commit**: with `overwrite`, `rename(part,
  target)`; without, an existence check then `rename` (no hard-link no-clobber: FAT, the RC-5's file
  system, has no hard links; the check-then-rename window is accepted, single user) → `sync_dir`
  → `done {path, clipped}` → `on_done()` (the IPC layer remembers the RC-5 memory there).
  On any error or cancel: remove the part file, then `remove_dir` (never recursive) the folders this
  job created, deepest first, so only empty folders of ours go; then `failed {message}` or
  `cancelled`. Cancel during `finishing` still wins until the rename.
- Nothing is ever written under the repository root (requirement 6): sources are only read, and
  destinations inside the root are refused. The only repository write that can happen during an
  export is the Player's own pending `player.json` flush that `TempoStore::load` triggers (it would
  happen 500 ms later anyway).
- **Logs** (requirement 9), prefix `calliope: `:
  `export start job=<j> id=<id> target=<rc5|mp3|ogg> path=<p> rate=<hz> channels=<c> count_in=<bpm> first_beat_ms=<ms> lead_frames=<n> overwrite=<b>`,
  `export done job=<j> path=<p> frames=<n> clipped=<n>`, `export failed job=<j|none> message=<m>`,
  `export cancelled job=<j>`,
  `export refused target=<t> reason=<memory|tools|encoder|not-folder|no-roland|inside-repo|is-folder|exists|busy|no-backing|no-tempo|stale>`.

### 2.7 Remembered folders, memory suggestion, dialogs (requirement 10)

- `Settings` gains (lenient load like `last_import_dir`; absent/`null`/invalid = `None`):
  `rc5_dir: Option<PathBuf>` (absolute), `export_dir: Option<PathBuf>` (absolute),
  `rc5_memory: Option<u8>` (1..=99). Setters `set_rc5_dir`, `set_export_dir`, `set_rc5_memory`.
- `rc5_dir` is saved when a picked base passes the checks of 2.5 (a plan was made); `export_dir`
  when a picked MP3/OGG folder passes its checks; `rc5_memory` when an RC-5 export is **done**
  (open question 6). Logs: `calliope: export remembered kind=<rc5|file> dir=<p>`,
  `calliope: export remembered memory=<n>`.
- The suggestion: `export_rules::suggest_memory(last: Option<u8>) -> u8` = 1 when `None`, else
  `min(last + 1, 99)`.
- Dialog start folder: `export_rules::start_dir(remembered, home) -> (from, dir)`: the remembered
  folder when it is a folder (`from=last`), else home (`from=home`), else none. Unlike
  `last_import_dir`, a missing folder is **not** forgotten: an unplugged RC-5 is normal, and the next
  time it is mounted at the same place the dialog opens there again. Log:
  `calliope: dialog start kind=<export-rc5|export-audio> from=<last|home|none> dir=<p|none>`.
- `DialogKind` gains `ExportRc5` (`"export-rc5"`, title "Choose the RC-5 drive (its root folder)")
  and `ExportAudio` (`"export-audio"`, title "Choose a folder for the MP3/OGG file"), both folder
  dialogs. The scripted picker answers them like every other kind.

### 2.8 IPC (`src/export_ipc.rs`; registration in `src/gui.rs`, `build.rs`, `capabilities/main.json`)

| Command | Does |
|---------|------|
| `export_info(id) -> ExportInfo` | `{id, available, reason, source, count_in: {bpm, first_beat_ms, lead_ms} \| null, memory}`: `export_source` under the repository lock, then the tempo map through the `TempoStore` (`lead_ms = 4 · 60000 / bpm`; an empty map → `available: false` with the no-tempo reason), `memory = suggest_memory(settings.rc5_memory)` |
| `prepare_export(id, target: TargetReq) -> Option<ExportPlan>` | memory check (RC-5), tools and encoder check, source and tempo check, folder dialog from the remembered folder, `ExportState::plan`, remember the folder. `None` = the dialog was cancelled (log `calliope: dialog ... result=cancelled`); `Err(message)` = refused |
| `start_export(token, overwrite, events) -> ExportSnapshot` | re-reads the tempo map through the `TempoStore`, `ExportState::start`; `on_done` saves `rc5_memory` for an RC-5 export |
| `cancel_export(job) -> Option<ExportSnapshot>` | cancel; a finished or unknown job is a no-op |
| `get_export() -> Option<ExportSnapshot>` / `watch_export(events)` | re-attach after a webview reload |

All are `async` with the work in `spawn_blocking`; argument names are single words. The frontend
still never sends a path: the folder comes from the Rust dialog, and `ExportPlan.path` is for
display only (like `pick_import_file`'s path). `gui.rs` manages `ExportState::new(discover)` (with
`with_pace` from `CALLIOPE_E2E_EXPORT_PACE` only under `e2e-hooks`) and calls
`export.shutdown(SHUTDOWN_WAIT)` on `RunEvent::Exit`. Testable functions `do_export_info`,
`do_prepare_export`, `do_start_export` take `&RepoState`, `&SettingsStore`, `&ExportState`,
`&dyn TempoStore` and `home`, like `ipc.rs`'s `do_*` functions.

### 2.9 The Export view (requirements 1, 2, 7, 8, 11)

`src/ui/views/ExportView.svelte`: `TrackBrowser heading="Export"` (read-only: no `editable`) and
`src/ui/components/export/ExportPane.svelte`. On mount: `load()` the library like the Editor (unless
a Library edit is open) and `attachExport()` (`watch_export`). Ctrl+F focuses the search.

**Target rule** (`src/ui/lib/export-format.ts`, pure): `exportTarget(treeFocus, selected)`:
when the tree's focused row (`lib.treeFocus`, which keeps the last focused row after the focus
leaves the tree) is a band or album key (`b:…`), the target is that group and nothing is enabled
("Select a track to export it"); otherwise it is the selected track (`lib.selectedId`), or none
("Select a track to export it"). The selection stays shared with the other views (open question 8).

Pane, top to bottom (nothing moves when state changes; every message line has reserved space):

1. Read-only rows Band, Album, Title (borderless view-mode style), the source line
   ("Backing track: backings/backing.flac") and the count-in line
   ("Count-in: 4 beats at 120 BPM, ending on the first beat (adds 2.0 s)"; without a tempo map the
   line is empty and the reason line below says "Set a tempo in the Player first: the export starts
   with a count-in").
2. A bordered group with the legend "Export...": row 1 "to BOSS RC-5" button, the label "Memory"
   and a 4 rem text box (`inputmode="numeric"`, prefilled with `info.memory`, refilled after each
   successful RC-5 export); row 2 "MP3 file"; row 3 "OGG file". Buttons 3.25 rem high. Below, the
   reason line (`role="status"`, muted) when disabled.
3. The job area (reserved two lines): while running, `Progress` + "Exporting <title> to <path> ·
   42 %" and a Cancel button (the focus moves to Cancel when an export starts); done:
   "Written to <path>." (RC-5: + " Eject the RC-5 before unplugging it."); failed:
   "Export failed: <message>" (accent text colour per the UI guide); cancelled: "Export cancelled;
   nothing was written."; a refusal from `prepare_export` shows its message the same way.

Buttons are disabled when the target is not an available track (no backing, or no tempo map), while `prepare_export` is waiting
for the dialog, and while an export runs (every track; the reason line says "An export is running").

Flow (`src/ui/lib/export-state.svelte.ts`): click → `prepareExport(id, target)` → `null`: nothing
(log) → plan with `exists` → `ConfirmDialog` "Replace the existing file <path>? Its old content
will be lost." (focus on No) → No: nothing is written (log) → Yes or no file: `startExport(token,
exists, onEvent)`. Events update the snapshot; `done` reloads `export_info` (new memory
suggestion).

Frontend log lines (`calliope-ui: `): `export target id=<id|none> kind=<track|group|none>
available=<b>`, `export prepare target=<t> result=<plan|cancelled|refused> exists=<b>`,
`export confirm answer=<yes|no> path=<p>`, `export phase=<p>`, `export done path=<p>`,
`export failed message=<m>`, `export layout overflow=<px> width=<px>` (after the size settles, like
the Player).

### 2.10 Navigation (requirement 11)

`VIEWS` becomes Library 1, Import 2, Editor 3, Playlists 4, Player 5, **Export 6** (id `export`,
summary "Copy a track's backing to the BOSS RC-5, or save it as MP3 or OGG.", features
`['gui-export']`), Settings 7; `viewForKey` accepts `Digit1`..`Digit7` / `Numpad1`..`Numpad7`.
NavBar icon: lucide `file-output`. The GUI tests that press `alt+6` for Settings move to `alt+7`.

### 2.11 Error and edge cases

| Case | Behaviour |
|------|-----------|
| memory 0, 100, "abc", empty | refused before any dialog, "Enter a memory number from 1 to 99" |
| base without `ROLAND` | refused after the dialog, "No ROLAND folder in <base>: is the RC-5 mounted?", nothing created |
| RC-5 unmounted between plan and start | `start` re-checks: "<base> is not an existing folder" |
| `WAVE` exists as a file | refused "<base>/ROLAND/WAVE is not a folder" |
| target exists, user says No | nothing written, the old file unchanged |
| destination read-only | `start` fails "Cannot write to <dir>: Permission denied (os error 13)", nothing left |
| disk full mid-export | failure with the OS reason, part and created folders removed |
| source file vanished | decoder failure with ffmpeg's last stderr line |
| source with > 2 channels | downmixed to stereo by ffmpeg |
| ffmpeg without libmp3lame/libvorbis | refused before the dialog with the encoder message |
| app exits mid-export | `shutdown` cancels, part removed (≤ 2 s) |
| no tempo map (or emptied before Start) | all three buttons disabled with the tempo reason; Rust refuses `no-tempo`, nothing written |
| a second export while one runs | refused "An export is running" (buttons disabled anyway) |

### 2.12 Test fixtures

`tests/fixtures/export/make-fixtures.sh` generates `tests/fixtures/library-export/` (invented content,
deterministic, `-fflags +bitexact`, like `player/make-fixtures.sh`). Band "The Example Band", album
"Export Tests" unless noted, all audio sines. Every exportable track has a `player.json`
(`{"schema_version": 6, "tempo": {"beats_per_bar": 4, "segments": [{"start_ms": S, "bpm": T}]}}`),
since a track without a tempo cannot be exported:

| id `0199c0a0-0000-7000-8000-000000000NNN` | Title | Type | Audio | Extras |
|------|-------|------|-------|--------|
| 401 | Stem Mix | stem | `backings/backing.flac` 44.1 kHz stereo 16-bit 4 s, 330 Hz left / 550 Hz right; `stems/guitar.flac` | `position` 3, `album_position` 1, `year` 2021; tempo `{0, 120}` |
| 402 | No Mix | stem | `stems/guitar.flac` only, no `backings` | tempo `{0, 120}` (disabled for the mix, not the tempo) |
| 403 | Plain MP3 | backing | `backing.mp3` 44.1 kHz stereo 4 s 440 Hz | tempo `{0, 120}` |
| 404 | Mono Backing | backing | `backing.flac` 44.1 kHz **mono** 4 s 660 Hz | tempo `{0, 120}` |
| 405 | High Rate | backing | `backing.flac` **48 kHz** stereo 4 s 1000 Hz | tempo `{0, 100}` (beat = 26460 frames at 44.1 kHz, 28800 at 48 kHz) |
| 406 | With Tempo | stem | `backings/backing.flac` 44.1 kHz stereo 4 s 440 Hz | tempo `{500, 120}` |
| 407 | Long Intro | backing | `intro.flac` 44.1 kHz stereo 6 s: 3 s digital silence, then 3 s 440 Hz | tempo `{3000, 100}` |
| 408 | Missing Mix | stem | `backings` lists `backings/backing.flac`, file absent | tempo `{0, 120}` |
| 409 | `Live: A/B?` | backing | `backing.flac` 44.1 kHz stereo 1 s | band `AC/DC*`, album "" , `position` 5 (no track tag expected); tempo `{0, 120}` |
| 410 | No Tempo | stem | `backings/backing.flac` 44.1 kHz stereo 4 s 440 Hz | **no** `player.json` (disabled: no tempo) |

## 3. Tasks

Rust tasks end with `cargo clippy -p calliope-gui --all-targets -- -D warnings` clean (and
`-p calliope-lib` for task 4), frontend tasks with `npm run check` clean (both in
`src/calliope-gui/`). Implementers run only the tests named in their task; the orchestrator runs
`npm test` and `npm run test:gui`. Tests use temporary directories only (copies of
`tests/fixtures/library-export/`, temp destinations), never the user's repository, `~`, a real
dialog, a real drive or another host. Extend existing tests instead of adding near-duplicates.
Never commit in a routine task.

### Task 1: Settings fields and dialog kinds
- **files:** `src/calliope-gui/src/settings.rs`, `src/calliope-gui/src/picker.rs`
- **does:**
  - `Settings`: after `last_import_dir`, add
    ```rust
    /// Root of the RC-5 drive chosen last (the export's folder dialog starts there); `None` = none yet.
    pub rc5_dir: Option<PathBuf>,
    /// Folder of the last MP3/OGG export; `None` = none yet.
    pub export_dir: Option<PathBuf>,
    /// RC-5 memory of the last successful export (1..=99); the next one suggests it plus one.
    pub rc5_memory: Option<u8>,
    ```
  - `load`: after the `last_import_dir` match, two matches for `rc5_dir` and `export_dir` with the
    exact pattern of `last_import_dir` (absolute path string, else a warning naming the field and
    "using none"), and for `rc5_memory`:
    ```rust
    match obj.get("rc5_memory") {
        None | Some(serde_json::Value::Null) => {}
        Some(serde_json::Value::Number(n)) if n.as_u64().is_some_and(|m| (1..=99).contains(&m)) => {
            s.rc5_memory = n.as_u64().map(|m| m as u8);
        }
        Some(v) => eprintln!("calliope: invalid rc5_memory {v} in {} (not 1..99); using none", path.display()),
    }
    ```
  - `SettingsStore`: `set_rc5_dir(&self, dir: Option<PathBuf>)`, `set_export_dir(&self, dir: Option<PathBuf>)`,
    `set_rc5_memory(&self, n: Option<u8>)`, each `-> std::io::Result<Settings>` with the body of
    `set_last_import_dir`.
  - Tests: `json_shape` expects `…,"last_import_dir":null,"rc5_dir":null,"export_dir":null,"rc5_memory":null}`.
    New `export_fields_roundtrip_and_lenient`: save with all three set, `load` equals; a file with
    `"rc5_dir":"rel"`, `"export_dir":5`, `"rc5_memory":0` (and separately `100`, `"7"`) loads them as
    `None` and keeps `"theme":"light"`; each setter persists across `SettingsStore::open`.
  - `picker.rs`: `DialogKind` gains `ExportRc5` (`"export-rc5"`) and `ExportAudio`
    (`"export-audio"`), added to `as_str` and the `parse` list. Extend `scripted_knows_import_kinds`
    with `export-rc5 /tmp/rc5` and `export-audio CANCEL` answered through `pick_folder`.
- **done when:** `cargo test -p calliope-gui --lib settings::` and `cargo test -p calliope-gui --lib picker::` pass.
- **routine:** yes

### Task 2: Export fixtures
- **files:** `src/calliope-gui/tests/fixtures/export/make-fixtures.sh` (new),
  `src/calliope-gui/tests/fixtures/library-export/` (generated, committed)
- **does:** a bash script modelled on `tests/fixtures/player/make-fixtures.sh` (same `ff` helper with
  `-fflags +bitexact -flags:a +bitexact`, idempotent, `FORCE=1` regenerates, `track.json` written by a
  `track` function) that creates the nine tracks of 2.12 exactly (ids, titles, types, rates, channels,
  durations, frequencies, positions, year, and the `player.json` of every track but 410, with the
  `{start_ms, bpm}` of the table), plus `calliope-repository.json` `{"schema_version": 1}`. The
  MP3 is `-c:a libmp3lame -q:a 4`. 407's silence is `anullsrc` 3 s concatenated with a 3 s sine
  (`-filter_complex concat`). Run it once; the output is committed with the feature.
- **done when:** `bash src/calliope-gui/tests/fixtures/export/make-fixtures.sh` succeeds, and a second run
  changes no file (`find src/calliope-gui/tests/fixtures/library-export -type f -exec sha256sum {} + | sort`
  is the same before and after it); `ffprobe` shows 405 at `sample_rate=48000`, 404 at `channels=1`;
  `du -sh src/calliope-gui/tests/fixtures/library-export` is under 2 MB;
  `cargo test -p calliope-gui --lib repository::` still passes.
- **routine:** yes

### Task 3: Shared WAV writer
- **files:** `src/calliope-gui/src/wav.rs` (new), `src/calliope-gui/src/audio_out.rs`, `src/calliope-gui/src/lib.rs`
- **does:** move `struct WavWriter` and its `impl` unchanged from `audio_out.rs` into `wav.rs` as
  `pub(crate)` (with its `use` lines), add `pub(crate) fn from_file(file: File, rate: u32) -> io::Result<Self>`
  (the header-writing body of `create`, which then becomes `Self::from_file(File::create(path)?, rate)`),
  and make `write`/`finish` `pub(crate)`. `lib.rs`: `mod wav;`. `audio_out.rs`: delete the moved
  block and add `use crate::wav::WavWriter;`. Tests in `wav.rs`: a 3-frame file is 44 + 24 bytes,
  bytes 0..4 `RIFF`, 4..8 = 60, 8..16 `WAVEfmt `, 16..20 = 16, format 3, channels 2, rate 44100,
  byte rate 352800, align 8, bits 32, 36..40 `data`, 40..44 = 24, then the samples as f32 LE.
- **done when:** `cargo test -p calliope-gui --lib wav::` and `cargo test -p calliope-gui --lib audio_out::` pass.
- **routine:** yes

### Task 4: Process runner with a piped stdin
- **files:** `src/calliope-lib/src/process.rs`
- **does:** `spawn_with` gets a `stdin: Stdio` parameter and returns `(Running, Option<ChildStdin>)`;
  `spawn`/`spawn_raw_stdout` pass `Stdio::null()` and keep their signatures; new
  `pub fn spawn_with_stdin(program, args, cwd, on_stdout_line, on_stderr_line) -> io::Result<(Running, ChildStdin)>`
  with `Stdio::piped()`. Update the module comment ("stdin closed unless …").
  Tests: `/bin/sh -c 'wc -c'` receives 100 000 bytes, stdin dropped, stdout line `100000`, exit 0;
  `sleep 60` with stdin: `cancel()` then `wait` returns within 3 s and a later write gives
  `BrokenPipe`; the existing tests unchanged.
- **done when:** `cargo test -p calliope-lib process` passes; `cargo clippy -p calliope-lib --all-targets -- -D warnings` clean.
- **routine:** no

### Task 5: Export rules (pure)
- **files:** `src/calliope-gui/src/export_rules.rs` (new), `src/calliope-gui/src/lib.rs`,
  `src/calliope-gui/src/repository.rs` (only `sanitize_component`, `cut_bytes`, `inside_root` → `pub(crate) fn`)
- **does:** exactly the functions of 2.2-2.5 and 2.7: `SourceKind`, `ExportSource`, `export_source`,
  `parse_memory`, `rc5_folder_name(n) -> "007_1"`, `rc5_target(base, n)`, `file_name(band, title, ext)`,
  `StreamInfo`, `parse_stream_info`, `output_format(kind, &StreamInfo) -> (u32, u32)` (with a local
  `enum Format { Rc5, Mp3, Ogg }`, or the job's `ExportKind` declared here and re-exported by the job),
  `tags(&TrackRecord)`, `has_encoder(listing, name)`, `suggest_memory`, `start_dir`,
  `NO_TEMPO_REASON: &str = "Set a tempo in the Player first: the export starts with a count-in"`, and
  `ExportKind { Rc5, Mp3, Ogg }` (`Serialize`/`Deserialize`, lowercase), which `output_format` takes and
  the job re-exports. std + serde only. `lib.rs`: `pub mod export_rules;`.
- **tests** (in the module): every row of the 2.2 table (build `TrackRecord`s by hand); `parse_memory`
  for "1", " 7 ", "007", "99" (ok) and "0", "100", "abc", "", "7.5", "-3", "1000" (the exact message);
  `rc5_target("/b", 7)` = `/b/ROLAND/WAVE/007_1/007_1.WAV` and 99 → `099_1`; `file_name` for
  ("AC/DC*", "Live: A/B?") → `AC_DC_ - Live_ A_B_.mp3`, ("", "T") → `T.ogg`, a 300-byte title cut on
  a UTF-8 boundary; `parse_stream_info` on real ffprobe JSON samples (FLAC 48000/1, MP3 with only a
  format duration, garbage → `None`); `output_format` for 44100/2, 48000/1, 22050/2, 96000/2, 88200/2,
  37800/1, 6 channels, for each target; `tags` with and without album/position/year and with a control
  character; `has_encoder` on a sample listing (present, absent, name only as a description word);
  `suggest_memory(None/Some(7)/Some(99))` = 1/8/99; `start_dir` with an existing remembered folder, a
  missing one (→ home, and nothing else changes), no home.
- **done when:** `cargo test -p calliope-gui --lib export_rules::` passes; clippy clean.
- **routine:** yes

### Task 6: Count-in and the audio pipeline
- **files:** `src/calliope-gui/src/export_audio.rs` (new), `src/calliope-gui/src/lib.rs`
- **does:** (`lib.rs`: `pub mod export_audio;`) `TICK_BEATS`, `CountInPlan`, `count_in_plan`, `mix_ticks` (2.3); `decoder_args`,
  `encoder_args` (2.4); `run(p: Pipeline, out: Output, cancel: &AtomicBool, progress: &mut dyn FnMut(u64))
  -> Result<RunStats { frames, clipped }, String>` with `Output::Wav(WavWriter)` |
  `Output::Encode { ffmpeg, args }` (encoder started first with `spawn_with_stdin`, then the decoder
  with `spawn_raw_stdout`; the `lead` silent frames are written first; 50 ms receive timeout and cancel as in
  2.4; pace sleeping; error texts "Cannot decode <file>: <last stderr>", "Cannot encode: <last stderr>",
  "Cannot write <part>: <io error>", `"cancelled"` for a cancel).
- **tests** (module, temp dirs, the real local ffmpeg on `tests/fixtures/library-export`):
  - `count_in_plan`: the two worked examples of 2.3 exactly (beat, lead, ticks, first-beat frame);
    empty map → `None`; first beat at 0 → ticks at 0, beat, …, and the first beat at `lead`; an odd
    beat (e.g. 120.7 BPM at 44.1 kHz) rounds the half-beat ticks half away from zero; 300 and 20 BPM;
    48 kHz; `beats_per_bar: 3` gives the same plan; a later first beat moves every tick by the same
    frames and never changes `lead`.
  - `mix_ticks`: block boundaries inside a click give the same result as one block; samples away from
    ticks unchanged bit for bit; a 0.9 signal under a tick is clamped and counted.
  - `run` to WAV: for 401 and 406 (44.1 kHz) the output equals, sample for sample, the expected
    buffer built in the test as `lead` zero frames ++ `ffmpeg … -f f32le -ac 2 -ar 44100` of the source,
    passed through `mix_ticks` (so the ticks are at the planned frames and everything else is the
    original bit for bit); 406: 264600 frames; 407: the ticks fall in silence (onsets found by a
    1500 Hz energy scan at exactly the six frames), the 440 Hz tone starts at frame 238140; 404 (mono):
    left == right; 405 (48 kHz): frames within 1 of `105840 + round(4 · 44100)` and the 1000 Hz tone
    dominant after the lead (Goertzel).
  - `run` to MP3/OGG for 401 and 406: ffmpeg decodes both back to 4 s + 2 s (4 beats at 120 BPM) ± 30 ms;
    `ffprobe` shows title/artist/album/track 3/date 2021.
  - a fake `ffmpeg` script (records its argv to a file, `sleep 60`) → `run` cancelled after 300 ms
    returns `"cancelled"` within 3 s and no child is left (`/proc/<pid>`), as in
    `player_source::cancelled_ffmpeg_returns_promptly_and_leaves_no_child`; a fake encoder that exits 1
    with "No space left on device" on stderr → the error names it.
  - `encoder_args` contains `-y`, `file:<part>`, `-map_metadata -1`, `-q:a 2` / `-q:a 6`, and no `-n`.
- **done when:** `cargo test -p calliope-gui --lib export_audio::` passes (under 20 s); clippy clean.
- **routine:** no

### Task 7: The export job
> **Orchestrator note (after tasks 1-5):** `export_rules::file_name` takes a 4th argument, the fallback
> (call it with `&t.id`); `start_dir(...)` returns `(&'static str, Option<PathBuf>)` with "last"/"home"/"none";
> `parse_stream_info` returns `None` without a positive duration; `ExportKind::as_str()` exists.

- **files:** `src/calliope-gui/src/export_job.rs` (new), `src/calliope-gui/src/export_job_tests.rs` (new), `src/calliope-gui/src/lib.rs`
- **does:** (`lib.rs`: `pub mod export_job;`; at the end of `export_job.rs`, `#[cfg(test)] #[path = "export_job_tests.rs"] mod tests;`
  like `editor.rs`) the types and `ExportState` of 2.6: `plan`, `start` (re-checks, created folders, part file),
  the thread (events, throttle, finishing, commit, cleanup, `on_done`), `cancel`, `snapshot`, `watch`,
  `shutdown`, the log lines of 2.6.
- **tests** (`export_job_tests.rs`, a `Rig` with a temp copy of `library-export`, a temp destination
  base and `Tools` from `PATH`; polling the snapshot every 20 ms):
  - `rc5_writes_the_memory_file`: base with `ROLAND/`, memory 7, track 401 → events writing… →
    finishing → done; `<base>/ROLAND/WAVE/007_1/007_1.WAV` is a 44-byte-header float WAV of
    88200 + 176400 frames;
    `WAVE` and `007_1` were created; no `.part` anywhere under the base; `on_done` ran once.
  - `rc5_refusals_write_nothing`: no `ROLAND` (exact message), base missing, `WAVE` a file, target a
    folder, base inside the repository root → `plan` errs and the base tree is byte-identical
    (`snapshot` helper) and has no new folders.
  - `overwrite_rules`: an existing `007_1.WAV` plus a sibling `notes.txt` → `exists: true`;
    `start(.., false, ..)` refused, file unchanged; `start(.., true, ..)` replaces it, `notes.txt`
    unchanged.
  - `mp3_and_ogg_names_and_tags`: 401 and 409 to a temp folder → `The Example Band - Stem Mix.mp3`,
    `AC_DC_ - Live_ A_B_.ogg` (409 without a track tag).
  - `cancel_leaves_nothing`: `with_pace(5.0)`, cancel after the first `writing` event → `cancelled`,
    no part, the folders it created removed, an already existing `WAVE` kept.
  - `failure_leaves_nothing`: a `chmod 0555` destination (skipped as root) → `start` errs with
    "Permission denied"; a fake encoder failing mid-way → `failed` with its message, no part.
  - `one_at_a_time_and_stale_tokens`: `plan` while running → "An export is running"; a used token
    → the stale message.
  - `no_tempo_is_refused`: 410 → `plan` errs with `NO_TEMPO_REASON`; a plan for 406 then `start` with an
    empty map → refused, nothing created; `start` with a map whose first beat moved since `plan` uses
    the new ticks (the WAV's tick onsets follow the new map).
  - `the_repository_is_untouched`: after every test above, `snapshot(<repo>)` equals the copy made at
    the start (`track.json`, the track folders, `player.json`).
  - `shutdown_mid_export`: `shutdown(2 s)` returns true, no part.
- **done when:** `cargo test -p calliope-gui --lib export_job` passes (under 30 s); clippy clean.
- **routine:** no

### Task 8: IPC commands and registration
- **files:** `src/calliope-gui/src/export_ipc.rs` (new), `src/calliope-gui/src/lib.rs`, `src/calliope-gui/src/gui.rs`,
  `src/calliope-gui/build.rs`, `src/calliope-gui/capabilities/main.json`
- **does:** (`lib.rs`: `pub mod export_ipc;`; the `do_*` functions and their argument types are `pub` for
  the acceptance tests) `ExportInfo`, `CountInInfo`, `do_export_info`, `do_prepare_export` (order: busy, memory,
  tools, encoder, source, tempo, dialog with start folder + `dialog start` log, plan, remember), `do_start_export`
  (re-reads the tempo through the `TempoStore`; the `on_done` closure saves `rc5_memory`), the six Tauri commands of 2.8, `ExportState` managed in
  `gui.rs` (pace from `CALLIOPE_E2E_EXPORT_PACE` under `#[cfg(feature = "e2e-hooks")]`), shutdown in
  `RunEvent::Exit`. Registration in the three places, in the same order, after `player_ipc::player_loop`:
  `export_ipc::export_info, prepare_export, start_export, cancel_export, get_export, watch_export`
  (`"export_info"`, … and `"allow-export-info"`, …).
- **tests** (module, a `RepoState` with an `Arc<ScriptedPicker>`, a temp `SettingsStore`,
  `FileTempoStore`): `info_for_each_fixture` (available/reason/count-in/memory for 401-410; 410 has the
  no-tempo reason, 402/408 the backing reasons); `prepare_refuses_no_tempo` (410: `Err`, no dialog);
  `start_rereads_the_tempo` (the map changed in `player.json` between prepare and start is used);
  `prepare_checks_before_the_dialog` (memory "abc" and a missing encoder through a fake `ffmpeg` that
  lists no encoders: `Err`, and `starts()` is empty); `prepare_uses_and_remembers_folders` (start
  folder = home first, then the remembered base; a missing remembered base → home and the setting kept;
  a refused base is not remembered); `memory_is_remembered_after_done` (7 → suggestion 8, then 99 stays 99
  via a settings file holding 99); `dialog_cancel_is_none`.
- **done when:** `cargo test -p calliope-gui --lib export_ipc::` and `cargo test -p calliope-gui --test frontend`
  pass; clippy clean.
- **routine:** no

### Task 9: Headless acceptance tests
- **files:** `src/calliope-gui/tests/acceptance_export.rs` (new)
- **does:** one `ac_*` test per Rust-side acceptance criterion (section 7), through the `do_*` functions
  of task 8 with temp dirs only; audio assertions by decoding with the local ffmpeg/ffprobe and reading the
  WAV header bytes; each test also asserts the repository copy is unchanged (`track.json` and the track
  folder). "After a restart" = reopening the `SettingsStore` from the same file. Tests:
  `ac_stem_track_with_mix_is_exportable`, `ac_stem_track_without_mix_is_not_exportable`,
  `ac_track_without_tempo_is_not_exportable` (410),
  `ac_plain_backing_is_exportable`, `ac_rc5_memory_7`, `ac_rc5_without_roland`, `ac_rc5_overwrite_confirmed`,
  `ac_rc5_mono_becomes_stereo`, `ac_rc5_bad_memory_numbers`, `ac_rc5_resamples_48k`, `ac_mp3_export`,
  `ac_ogg_export`, `ac_overwrite_declined`, `ac_cancel_leaves_nothing` (`with_pace`),
  `ac_unwritable_destination` (skipped as root), `ac_remembered_after_restart`,
  `ac_count_in_before_the_first_beat` (406 and 407, WAV and MP3). Every length/duration assertion is
  "the original + 4 beats at the first section's BPM" (owner answer 1).
- **done when:** `cargo test -p calliope-gui --test acceptance_export` passes (under 60 s).
- **routine:** no

### Task 10: Navigation entry
- **files:** `src/calliope-gui/src/ui/lib/views.ts`, `src/calliope-gui/src/ui/lib/views.test.ts`, `src/calliope-gui/src/ui/components/NavBar.svelte`
- **does:** `ViewId` gains `'export'`; in `VIEWS`, after `player`, insert
  `{ id: 'export', label: 'Export', digit: 6, summary: "Copy a track's backing to the BOSS RC-5, or save it as MP3 or OGG.", features: ['gui-export'] }`
  and change Settings to `digit: 7`; in `viewForKey` the regex becomes `/^(?:Digit|Numpad)([1-7])$/`.
  NavBar: `import FileOutputIcon from '@lucide/svelte/icons/file-output';` and `export: FileOutputIcon` in `icons`.
  `views.test.ts`: labels `[..., 'Player', 'Export', 'Settings']`, digits `[1..7]`, `Digit6` → `'export'`,
  `Digit7` → `'settings'`, the rejected key becomes `Digit8`.
- **done when:** `npx vitest run src/ui/lib/views.test.ts` passes and `npm run check` is clean (App.svelte
  still renders Settings for `export` until task 11; that is fine).
- **routine:** yes

### Task 11: Export view placeholder in the app shell and the shortcut tests
- **files:** `src/calliope-gui/src/ui/views/ExportView.svelte` (new), `src/calliope-gui/src/ui/App.svelte`,
  `src/calliope-gui/src/ui/acceptance_frontend.test.ts`
- **does:** `ExportView.svelte` = the `PlaylistsView` pattern for `'export'` (replaced in task 15);
  `App.svelte`: import it and add `{:else if ui.view === 'export'}<ExportView />` before `{:else}`;
  `acceptance_frontend.test.ts`: `NAV_ORDER` gains `'Export'` after `'Player'`, the placeholder test
  accepts `Export` with `expect(text).toContain('gui-export')`, `Digit7` (the "does nothing" key) becomes
  `Digit8`.
- **done when:** `npx vitest run src/ui/acceptance_frontend.test.ts src/ui/App.test.ts` passes; `npm run check` clean.
- **routine:** yes

### Task 12: GUI tests use Alt+7 for Settings
- **files:** `src/calliope-gui/tests/gui_e2e.rs`, `src/calliope-gui/tests/gui_library_e2e.rs`, `src/calliope-gui/tests/gui_editor_e2e.rs`
- **does:** `gui_e2e.rs`: `const VIEWS: [&str; 7] = ["library", "import", "editor", "playlists", "player", "export", "settings"];`
  and `alt+6` → `alt+7` in `theme_switch_persists`; `gui_library_e2e.rs` line with `alt+6` → `alt+7`;
  `gui_editor_e2e.rs` comment "Settings, Alt+6" → "Settings, Alt+7".
- **done when:** `cargo test -p calliope-gui --features e2e-hooks --test gui_e2e --test gui_library_e2e --test gui_editor_e2e --no-run`
  compiles and `grep -n 'alt+6' src/calliope-gui/tests/gui_e2e.rs src/calliope-gui/tests/gui_library_e2e.rs` prints nothing.
- **routine:** yes

### Task 13: TS IPC types and wrappers
> **Orchestrator note (after tasks 6-9), the backend as built:**
> - Commands (async): `export_info(id)`, `prepare_export(id, target)` with `target` `{kind:'rc5', memory:'7'}` |
>   `{kind:'mp3'}` | `{kind:'ogg'}` -> `ExportPlan | null` (`{token, path, exists}`; null = dialog cancelled; a
>   refusal is a rejected string), `start_export(token, overwrite, events)`, `cancel_export(job)` -> snapshot | null,
>   `get_export()` / `watch_export(events)` -> `ExportSnapshot | null`.
> - `ExportInfo {id, available, reason: string|null, source: string|null, count_in: {bpm, first_beat_ms, lead_ms}|null,
>   memory: number}`; no tempo: `available:false`, `source` set, `count_in:null`, the no-tempo reason.
> - `ExportSnapshot {job, id, title, target: 'rc5'|'mp3'|'ogg', path, phase: 'writing'|'finishing'|'done'|'failed'|
>   'cancelled', progress, message: string|null, clipped}`; `ExportEvent` tagged by `phase`: `writing {progress}`,
>   `finishing`, `done {path, clipped}`, `failed {message}`, `cancelled`. `rc5_memory` is saved before `done` is sent.
> - Messages: stale token "This export is no longer ready; press the button again", busy "An export is running",
>   tools "Install ffmpeg to export (ffmpeg and ffprobe are needed)". E2E knob `CALLIOPE_E2E_EXPORT_PACE=<f>`
>   (e2e-hooks builds only): an export takes f x the audio's duration.

- **files:** `src/calliope-gui/src/ui/lib/ipc.ts` (append at the end), `src/calliope-gui/src/ui/lib/ipc-export.test.ts` (new)
- **does:** `Settings` gains optional `rc5_dir?`, `export_dir?: string | null`, `rc5_memory?: number | null`.
  Append a block `// Export: mirrors src/export_job.rs and src/export_ipc.rs.` with the types
  `ExportKind`, `ExportTargetReq` (`{kind:'rc5'; memory: string} | {kind:'mp3'} | {kind:'ogg'}`),
  `CountInInfo`, `ExportInfo`, `ExportPlan`, `ExportPhase`, `ExportSnapshot`, `ExportEvent` exactly as
  in 2.6/2.8, a private `exportChannel` (the `playerChannel` pattern) and the wrappers `exportInfo(id)`,
  `prepareExport(id, target)`, `startExport(token, overwrite, onEvent)`, `cancelExport(job)`,
  `getExport()`, `watchExport(onEvent)`.
  Test (the `ipc-import.test.ts` pattern): command names and argument names (`id`, `target`, `token`,
  `overwrite`, `events`, `job`), and a fake channel event reaching `onEvent`.
- **done when:** `npx vitest run src/ui/lib/ipc-export.test.ts` passes; `npm run check` clean.
- **routine:** yes

### Task 14: Export pane helpers (pure)
- **files:** `src/calliope-gui/src/ui/lib/export-format.ts` (new), `src/calliope-gui/src/ui/lib/export-format.test.ts` (new)
- **does:** `exportTarget(treeFocus, selected)` (2.9: `{kind:'group'} | {kind:'track', track} | {kind:'none'}`;
  a key starting with `b:` is a group), `progressText(snap)`
  ("Exporting <title> to <path> · 42 %"), `countInText(info)` = "Count-in: 4 beats at 120 BPM, ending on
  the first beat (adds 2.0 s)" or "" without a count-in, `resultText(snap)` (the done/failed/cancelled texts of 2.9,
  with the RC-5 eject sentence), `confirmText(path)`, `disabledReason(target, info, busy)`.
- **done when:** `npx vitest run src/ui/lib/export-format.test.ts` passes (each text, each target case,
  `b:x|a:y` and `b:x` are groups, a track key is not).
- **routine:** yes

### Task 15: Export state, view and pane
- **files:** `src/calliope-gui/src/ui/lib/export-state.svelte.ts` (new), `src/calliope-gui/src/ui/views/ExportView.svelte`,
  `src/calliope-gui/src/ui/components/export/ExportPane.svelte` (new), `src/calliope-gui/src/ui/views/ExportView.test.ts` (new)
- **does:** the state (`ex`: info, memory text, snapshot, message, pending confirmation; `loadInfo`,
  `exportTo(kind)`, `answerConfirm`, `cancel`, `attachExport`; frontend logs of 2.9), the view and the
  pane of 2.9 (`ConfirmDialog` reused; `Progress`; reserved lines; focus to Cancel on start; the
  `export layout` log).
- **tests** (vitest + mockIPC, fake channel events): buttons enabled for a stem track with a mix and for
  a plain backing; disabled with "No mix saved yet: save one in the Editor", with the no-tempo reason (`available: false`
  from `export_info`) and with the group reason;
  memory prefilled from `info.memory`; RC-5 click sends `{kind:'rc5', memory:'7'}`; a `null` plan does
  nothing; `exists: true` opens the confirmation naming the path, No sends no `start_export`, Yes sends
  `overwrite: true`; progress line and Cancel during `writing`, `cancel_export` on click; done text
  and the info reload; failed text; buttons disabled while running; a refusal's message shown.
- **done when:** `npx vitest run src/ui/views/ExportView.test.ts` passes; `npm run check` clean.
- **routine:** no

### Task 16: GUI end-to-end tests and visual review
> **Orchestrator note (after tasks 10-15), the UI as built:** `alt+6` opens Export, `alt+7` Settings; heading
> `h1#view-heading` "Export"; pane `section[aria-label="Export"]` with `[data-testid="export-pane"]`; fieldset legend
> "Export..."; buttons named exactly `to BOSS RC-5`, `MP3 file`, `OGG file`, `Cancel`; memory box `input#export-memory`
> ("Memory"); fields `#export-band`, `#export-album`, `#export-title`; text lines `export-source`, `export-count-in`,
> `export-reason` (role status), `export-progress`, `export-result` (status; alert on failure: "Written to <path>." +
> " Eject the RC-5 before unplugging it." for the RC-5, "Export failed: <m>", "Export cancelled; nothing was written.").
> Overwrite confirmation: an `alertdialog` "Replace the existing file <path>? Its old content will be lost.", focus on
> No. Focus moves to Cancel when an export starts. Log lines (`calliope-ui:`): `export target id= kind= available=`,
> `export prepare target= result=<plan|cancelled|refused> exists=`, `export confirm answer= path=`, `export phase=`,
> `export done path=`, `export failed message=`, `export layout overflow= width=`. `clipped` is a number.

- **files:** `src/calliope-gui/tests/gui_export_e2e.rs` (new), `src/calliope-gui/scripts/test-gui.sh`
  (`gui_export_e2e` added to `BINARIES`), `src/calliope-gui/tests/common/mod.rs` (an `export_dirs`
  helper copying `library-export` into the test's own `target/gui-e2e/<test>/` if needed)
- **does:** tests (scripted picker answers `export-rc5 <abs>` / `export-audio <abs>`, files asserted on
  disk, `snapshot` of the repository unchanged, `NullBackend` asserted as everywhere):
  - `export_view_buttons`: Alt+6 → `view=export`; select 401 → `available=true`; 402 → reason text;
    focus the band row → `kind=group`; 403 → enabled; 410 → disabled with the tempo reason; screenshots dark 1280x800 and light 1024x640
    (tree shown), and `export layout overflow` ≤ 14 px at 1280x800.
  - `export_rc5_flow`: base with `ROLAND`, memory 7 → `007_1/007_1.WAV` header bytes; restart the app
    → `dialog start kind=export-rc5 from=last` and the box shows 8; export 404 to 7 again → the
    confirmation (screenshot) naming the path, No → file unchanged, then Yes → replaced; the mono
    export is stereo with equal channels.
  - `export_rc5_refusals`: memory `abc` → message, no `dialog` line; a base without `ROLAND` → the
    "is the RC-5 mounted?" message, nothing created.
  - `export_mp3_and_ogg`: both files in the destination with the expected names; ffprobe tags; the
    dialog starts in the remembered folder the second time.
  - `export_cancel_and_failure`: `CALLIOPE_E2E_EXPORT_PACE=1`, export 407 (6 s + 2.4 s), wait for
    `export phase=writing`, press Cancel (keyboard: the focus is on it) → `export cancelled`, the
    destination is empty; then a `chmod 0555` destination → the failure text with "Permission denied"
    (skipped as root).
  - Visual review: the agent looks at the screenshots (idle, running, done, failed, confirmation, both
    themes, both sizes) and fixes clipping, contrast or jumping lines.
- **done when:** `npm run test:gui` passes (all binaries) and the screenshots in `target/gui-shots/export-*`
  have been reviewed.
- **routine:** no

### Task 17: README, UI guide, licences
- **files:** `src/calliope-gui/README.md`, `docs/ui.md`, `docs/licences.md`
- **does:** README: the shortcut line becomes "Alt+1 ... Alt+7: Library, Import, Editor, Playlists,
  Player, Export, Settings" and a short "Export" usage section (RC-5 root + memory, MP3/OGG folder,
  count-in rule: 4 beats of silence with six ticks ending on the first beat, a tempo is required;
  "eject before unplugging"). `docs/ui.md` "Decided by the team": the Export view layout
  of 2.9 and the navigation order with Export at Alt+6 / Settings at Alt+7 (amending the
  gui-frontend-foundation entry). `docs/licences.md`, ffmpeg row: "also MP3 (libmp3lame, LGPL) and Ogg
  Vorbis (libvorbis, BSD) encoding for exports, through the user's ffmpeg; nothing bundled".
- **done when:** `grep -n 'Alt+7' src/calliope-gui/README.md docs/ui.md` finds both, and `npm run check` is still clean.
- **routine:** yes

## 4. Test strategy

- **Unit (Rust):** `export_rules` (every rule and message, pure), `export_audio` (count-in frame arithmetic: one rounding of the beat, exact tick frames, lead = 4 beats,
  tick mixing, argv, the pipeline on the real local ffmpeg with generated fixtures, cancel with a fake
  ffmpeg script), `export_job_tests` (plans, start, commit, overwrite, cancel, failures, cleanup,
  shutdown, repository untouched), `export_ipc` (dialog start folders through the recording
  `ScriptedPicker`, settings remembered, checks before the dialog), `wav`, `settings`, `picker`,
  `calliope-lib` `process` (piped stdin).
- **Headless acceptance:** `tests/acceptance_export.rs`, one `ac_*` test per criterion (section 7).
- **Frontend:** vitest + mockIPC (`views.test.ts`, `acceptance_frontend.test.ts`, `ipc-export.test.ts`,
  `export-format.test.ts`, `ExportView.test.ts`).
- **GUI e2e:** `tests/gui_export_e2e.rs` on private X displays (`npm run test:gui`), scripted dialogs
  (`CALLIOPE_E2E_DIALOG_ANSWERS`), the pace knob for Cancel, files asserted on disk, screenshots for
  visual review (display `:1` with `gui-shot` for ad-hoc checks).
- **Stand-ins:** the **RC-5** is a temp folder with a `ROLAND/` subfolder (a "drive without ROLAND" is a
  plain temp folder; an "unwritable drive" is `chmod 0555`; "unmounted" is a folder removed between plan
  and start). **ffmpeg/ffprobe** are the real local tools (no network) on the committed fixtures; a
  "ffmpeg without encoders" and a "hanging/failing encoder" are small fake scripts in temp dirs. **Dialogs**
  are always the scripted picker. No test touches a real drive, `/run/media`, the user's repository or
  settings, an audio device, or the network. No symphonia.
- **Whole suite** (in `src/calliope-gui/`): `npm test` (headless, all crates), then `npm run test:gui`
  (or `DISPLAY=:1 npm run test:gui:serial`).

## 5. Deployment

Only the laptop app changes (`calliope-gui`, `npm run build:app`). `calliope-stems` and archserver are
untouched. `settings.json` gains three optional fields that older builds ignore. Run-time needs are
unchanged in kind: `ffmpeg`/`ffprobe` (Arch's `ffmpeg` package has libmp3lame and libvorbis).

## 6. Manual checks (owner, on the laptop with the real RC-5)

1. **RC-5 import.** Connect the RC-5 by USB so it mounts (e.g. `/run/media/<you>/BOSS_RC-5`). In Export
   select a track with a saved mix and a tempo, keep memory 7 (or an empty memory), press "to BOSS
   RC-5", choose the drive's root folder. "Written to …/ROLAND/WAVE/007_1/007_1.WAV" appears. Eject the
   drive (file manager or `udisksctl unmount`), unplug, select memory 7 on the RC-5 and play: the
   track plays after 4 beats, and the six count-in ticks lead exactly into the first beat in time with
   the song. If the
   RC-5 refuses the file or shows an error, report it (open question 7: a `fact` chunk may be needed).
2. **Not the root.** Press "to BOSS RC-5" again and choose the `ROLAND` folder itself (not the root):
   "No ROLAND folder in …: is the RC-5 mounted?" and nothing changes on the drive.
3. **Overwrite.** Export another track to the same memory: the confirmation names
   `…/007_1/007_1.WAV`; No leaves the old loop on the RC-5, Yes replaces it (check on the unit).
4. **Remembered.** Quit and restart Calliope: the memory box shows the last number plus one, and the
   folder dialog opens on the RC-5 root. With the RC-5 unplugged, it opens in your home folder; plug it
   back in and it opens on the drive again.
5. **No tempo.** Select a track whose tempo was never set (no BPM in the Player): the three buttons are
   disabled with "Set a tempo in the Player first…". Set a BPM in the Player, come back: they are enabled.
6. **MP3/OGG.** Export a track as MP3 and as OGG to `~/Music/exports`: the files are named
   "<Band> - <Title>", play in your usual player with the right title, band, album, track number and
   year, and the length shown is the track's plus 4 beats.
7. **Cancel and unplug.** Start an RC-5 export of a long track and press Cancel: the memory folder has
   no new file. Start another and pull the USB cable mid-way: the export fails with a reason, and
   after re-plugging no `.part` file is in the memory folder (a crash or power loss may leave a hidden
   `.…calliope-….part` there; delete it by hand).
8. **Listen.** Play the exported MP3 next to the Player: the backing sounds the same, the count-in is
   the Player's click.

## 7. Acceptance mapping

| Acceptance criterion | Tasks | Tests |
|---|---|---|
| Stem track with a saved mix (and a tempo map, required for the count-in) → the three buttons enabled | 5, 8, 14, 15, 16 | `export_source` table; `info_for_each_fixture`; `ac_stem_track_with_mix_is_exportable`; `ExportView.test.ts`; GUI `export_view_buttons` |
| Stem track without a mix → disabled, "no mix saved yet" | 5, 8, 14, 15, 16 | `export_source` table; `info_for_each_fixture` (402, and 408 for a missing mix file); `ac_stem_track_without_mix_is_not_exportable`; `ExportView.test.ts`; GUI `export_view_buttons` |
| Band or album selected → disabled | 14, 15, 16 | `export-format.test.ts` (`exportTarget`); `ExportView.test.ts`; GUI `export_view_buttons` (band row) |
| Plain backing track (with a tempo map) → enabled | 5, 8, 15, 16 | `export_source` table; `ac_plain_backing_is_exportable`; GUI (403) |
| `B` with `ROLAND`, memory 7 → `B/ROLAND/WAVE/007_1/007_1.WAV`, 32-bit float, stereo, 44.1 kHz, no metadata, same length and sound (= the original + 4 beats; the original unchanged after the lead, plus the ticks) | 2, 3, 5, 6, 7, 8 | `wav` header test; `run` WAV equality test (401, 406); `rc5_writes_the_memory_file`; `ac_rc5_memory_7`; GUI `export_rc5_flow` |
| `B` without `ROLAND` → refused "is the RC-5 mounted?", nothing written | 5, 7, 8, 16 | `rc5_refusals_write_nothing`; `ac_rc5_without_roland`; GUI `export_rc5_refusals` |
| Existing `007_1.WAV` → confirmation names it; after Yes it holds the new track | 7, 15, 16 | `overwrite_rules`; `ac_rc5_overwrite_confirmed`; `ExportView.test.ts`; GUI `export_rc5_flow` |
| Mono backing → stereo WAV, same signal on both channels | 5, 6 | `run` mono test (404); `ac_rc5_mono_becomes_stereo`; GUI `export_rc5_flow` |
| Memory 0, 100, "abc" → refused, nothing written | 5, 8, 16 | `parse_memory` tests; `prepare_checks_before_the_dialog`; `ac_rc5_bad_memory_numbers`; GUI `export_rc5_refusals` |
| 48 kHz backing → 44.1 kHz WAV, same duration (+ 4 beats) | 5, 6 | `output_format`; `run` 48 kHz test (405); `ac_rc5_resamples_48k` |
| `D`, MP3 → `<Band> - <Title>.mp3` tagged title/band/album, decodes to the duration (+ 4 beats) | 5, 6, 7, 16 | `file_name`, `tags`; `run` MP3 test; `mp3_and_ogg_names_and_tags`; `ac_mp3_export`; GUI `export_mp3_and_ogg` |
| `D`, OGG → Ogg Vorbis file, decodes to the duration (+ 4 beats) | 5, 6, 7, 16 | `run` OGG test; `ac_ogg_export`; GUI `export_mp3_and_ogg` |
| Target exists, the user cancels the confirmation → unchanged | 7, 15, 16 | `overwrite_rules` (`start` not called); `ExportView.test.ts` (No sends nothing); `ac_overwrite_declined`; GUI `export_rc5_flow` |
| Cancel during an export → no file left in the destination | 6, 7, 15, 16 | `run` cancel test; `cancel_leaves_nothing`; `ac_cancel_leaves_nothing`; GUI `export_cancel_and_failure` |
| Unwritable destination → fails with its reason, track unchanged | 7, 16 | `failure_leaves_nothing`; `ac_unwritable_destination`; GUI `export_cancel_and_failure` |
| Any export → the track's folder and `track.json` unchanged | 7, 9 | `the_repository_is_untouched`; every `ac_*` asserts it |
| (req. 9) started / done with the path / failed / cancelled logged | 7 | the log lines of 2.6, asserted in the GUI tests |
| (req. 10) folders remembered separately, dialog opens there, memory = last + 1, 99 stays 99 | 1, 5, 8, 16 | settings tests; `start_dir`, `suggest_memory`; `prepare_uses_and_remembers_folders`; `memory_is_remembered_after_done`; `ac_remembered_after_restart`; GUI `export_rc5_flow` |
| (req. 11) Export after Player (Alt+6), Settings Alt+7 | 10, 11, 12 | `views.test.ts`; `acceptance_frontend.test.ts`; GUI `shortcuts_switch_views` |
| (req. 12) count-in prepended: 4 beats of silence, six ticks ending on the first beat | 6, 7 | `count_in_plan`, `mix_ticks`, `run` tests on 406/407; `ac_count_in_before_the_first_beat` |
| (owner answer 2) no tempo map → export disabled with the tempo reason | 5, 7, 8, 14, 15, 16 | `no_tempo_is_refused`; `info_for_each_fixture` (410); `prepare_refuses_no_tempo`; `ac_track_without_tempo_is_not_exportable`; `ExportView.test.ts`; GUI `export_view_buttons` |

## 8. Open questions (answered by the owner 2026-10-23)

1. **Length with a count-in.** *Owner:* the exported file is **always** the original length + the
   count-in (exactly 4 beats at the first section's BPM); "same length / duration" in the criteria means
   original + 4 beats. (2.3; tasks 6, 7, 9, 16.)
2. **No tempo map.** *Owner:* export **disabled** (all three buttons), the pane says "Set a tempo in the
   Player first: the export starts with a count-in"; no export without a count-in. (2.3, 2.8, 2.9;
   tasks 2, 5, 7, 8, 9, 14, 15, 16.)
3. **Layout.** *Owner:* strictly on the song's beat grid: exactly 4 beats of silence before the
   original's 0:00, then the original unchanged (intro kept); ticks at output time
   `first_beat + {0, 1, 2, 2.5, 3, 3.5}` beats, ending on the first beat (output `lead + first_beat`),
   mixed over whatever is there. The first beat is `segments[0].start_ms`, read at export time (a later
   Editor feature will place it precisely). Exact frames at the target rate, the beat rounded once like
   the Player's `count_in_plan`. (2.3.)
4. **Tick sound.** *Owner:* the Player's click, no accent (as recommended).
5. **Tempo.** *Owner:* first section's BPM at 100 %, Player speed ignored, always 4 beats (as recommended).
6. **When things are remembered.** Recommended answer kept: the memory number after a successful RC-5
   export only; a folder when picked and accepted; a missing remembered RC-5 folder is kept.
7. **WAV chunks.** Recommended answer kept: `fmt ` (16 bytes) + `data`, no `fact`; verified on the
   real RC-5 (manual check 1), a follow-up adds `fact` if needed.
8. **"A band or an album is selected".** Recommended answer kept: the tree's focused band/album row
   disables everything in the Export view.
9. **MP3 rates and mono.** Recommended answer kept: MP3 ≤ 48 kHz (44.1 kHz for 88.2/176.4, else
   48 kHz), OGG keeps the rate, mono stays mono in MP3/OGG.
10. **Tags.** Recommended answer kept: ID3v2.3, no ID3v1, track number only with an album, year as `date`.
