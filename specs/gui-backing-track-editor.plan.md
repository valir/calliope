# Plan: Backing track editor (stem mixer in the Editor view)

Spec: `specs/gui-backing-track-editor.md`. Architecture: `docs/architecture.md`. UI guide: `docs/ui.md`.
Path conventions: as in `docs/architecture.md` ("Paths in this document"): `src/*.rs`, `src/ui/...`, `tests/...`,
`build.rs`, `capabilities/`, `package.json` are inside `src/calliope-gui/`.

Revision 2 (2026-10-08): owner answers applied. Time shown as `3:07.4`; volume in dB with boost and a clip indicator;
lane Play solos its stem; a never-saved track starts with all stems unchecked; playback stops when the track or the
view changes; `track.json` gets a list of named backing variants (this feature saves one); `original.flac` hidden;
FLAC confirmed; `symphonia` rejected for good.

## 1. Summary

The Editor view (Alt+3) gets the Library's track tree on the left and, on the right, a stem mixer for the selected
stem track. Each stem has a lane with its name, Play (solo), Unmute and a volume slider in dB. Below them, the Mix lane
has Play/Pause, Stop, a position slider, an editable `M:SS.T` time field, a clip indicator and Save. Playback and
mixing run in Rust: decoded stems in memory, a pure mixer, a transport state machine, and an audio output behind a
trait with a real `cpal` device and test fakes. The webview only shows state and sends commands. Save renders the mix
of the checked stems offline to `backings/backing.flac` in the track folder. It records the file in `track.json` as
the first entry of a new `backings` list of named variants, each with its mix settings.

## 2. Design

### 2.1 Where audio runs, and why (CSP unchanged)

- **Rust plays the audio; the webview never does.**
  - The architecture already says the frontend never keeps time and audio/MIDI/sync logic stays in Rust.
  - The overview wants an audio-output setting and, later, tight sync (Player, MIDI clock).
  - Web Audio in WebKitGTK goes through GStreamer, with its own latency and no device choice. It would also need
    `media-src`/`blob:` or large IPC byte transfers.

  With Rust-side audio, **the production CSP does not change**. The existing static CSP test in `tests/frontend.rs`
  keeps guarding it.
- **Output: `cpal` 0.18** (Apache-2.0). On Linux it uses the ALSA backend, which reaches PipeWire/PulseAudio through the
  ALSA default device. The system default output device is used; choosing a device is the later "Audio output"
  setting (gui-play-backing-track). Build prerequisite: `alsa-lib` (headers + `pkg-config`), present on Arch with any
  desktop.
- **Decoding: `claxon` 0.4** (Apache-2.0, pure-Rust FLAC decoder). Stems are FLAC by construction (import pipeline).
  **`symphonia` (MPL-2.0) is rejected by the owner, permanently**. The Player feature must play other formats another
  way (e.g. convert with ffmpeg to FLAC once, then decode with claxon).
- **Encoding: `flacenc` 0.5** (Apache-2.0, pure Rust), `default-features = false` (no thread pool, no `log`). The
  Editor doesn't need `ffmpeg`. In release builds it runs far faster than real time. The workspace `Cargo.toml` sets
  `[profile.dev.package.claxon]` and `[profile.dev.package.flacenc]` to `opt-level = 3`, so debug builds and tests stay
  fast.

### 2.2 Playback model: stems decoded into memory as 16-bit

When a stem track is opened in the Editor, every stem is decoded once into memory as interleaved `i16` (mono stays
mono).

- **Why:**
  - Seeks are sample-exact and instant (an index).
  - Mixing happens in the audio callback, with no decoder threads or ring buffers.
  - Volume, mute and solo changes take effect within one device buffer.
- **Cost:** about 10.6 MB per stereo stem-minute at 44.1 kHz (6 stems x 5 min = 318 MB; 6 stems at the 15-minute limit
  = 952 MB).
  - A cap of **1.5 GiB of decoded audio** per track refuses anything larger with a clear message.
  - Loading 6 x 5 min takes a few seconds in a release build and shows "Loading stems... 3 of 6".
- **Precision:** 24-bit stems are rounded to 16 bits **for listening only**. Save re-reads the files at full precision
  (2.5).

Stem rules checked at open (the message names the stem):
- every listed stem file exists and is FLAC;
- all stems have the **same sample rate** (else "The stems have different sample rates");
- 1 or 2 channels;
- 8..=24 bits.

Different lengths are allowed: the track duration is the longest stem, and shorter stems are silent after their end.

### 2.3 Gain, clipping and solo (identical in playback and Save)

- **Volume is a gain in dB**: slider from **-60 dB to +12 dB in 0.5 dB steps**. The bottom position (-60) means
  **Off** (-inf, silent). Default **0 dB** (unity = the stem as extracted), marked by a tick on the slider track.
  - Value text: `0.0 dB`, `+6.5 dB`, `-12.0 dB`, `Off` (also the slider's `aria-valuetext`).
  - Clicking the value text resets the lane to 0 dB.
  - Linear factor = `10^(dB/20)`, or 0 for Off. The checkbox decides whether the stem takes part (`unmuted`).
- **Clipping**:
  - **What happens to the samples:** the sum is **hard-clipped** at full scale (+/-1.0) by `mixer::mix`. That one
    function is used by playback and by Save, so both clip the same way. No limiter: a limiter changes the sound in
    ways that are hard to test exactly, and a visible warning lets the user fix the gains instead.
  - **How it is reported:** `mix` returns the number of clipped samples.
    - Playback: the audio callback adds them to an atomic counter. The ticker turns a rising counter into
      `clipping: true` in the transport state, held for 1 s after the last clipped sample (the hold is in Rust; the
      frontend keeps no time). The Mix lane shows an amber/red **CLIP** badge while it's true.
    - Save: the render counts clipped samples and the `saved` event carries `clipped_samples`. If it's > 0 the pane
      shows (inline, no pop-up): "Saved. N samples were clipped (about X s); lower some volumes and save again."
- **Solo (lane Play)** is a **transient listening state**. It never changes the other lanes' checkboxes or volumes
  and is never saved.
  - **Lane Play on stem X:**
    - checks X's Unmute (persisting, as the acceptance criterion requires);
    - sets `solo = X`;
    - presses Mix Play: playback starts if stopped or paused; if already playing it simply switches to the solo.

    While solo is on, the effective gain of every other stem is 0, and X plays at its own volume.
  - **Solo ends** (and the checked stems play again as the mix) when:
    1. the user presses **Mix Play** (start or resume): "Mix Play plays the mix";
    2. the user presses the soloed lane's button again (it reads "Solo" while active): solo off, playback continues
       with the mix;
    3. the user unchecks X's Unmute;
    4. **Stop**, the end of the track, a track change or leaving the Editor view.

    **Another lane's Play** moves the solo to that lane. **Mix Pause keeps the solo**, but the following Mix Play ends
    it; to resume the solo, press the lane's Play.
  - **UI while solo is on:**
    - the soloed lane's button reads **"Solo"**, amber, `aria-pressed="true"`;
    - the other lanes' controls are dimmed (still usable, their changes apply once solo ends);
    - the Mix lane shows "Solo: Guitar" next to the time field.
  - **Save during solo** saves the mix of the checked stems (solo is ignored).

### 2.4 Rust modules (all pure and unit-tested except the cpal glue and the Tauri wrappers)

| Module | Responsibility |
|---|---|
| `src/stem_audio.rs` | `probe(path) -> StemInfo {sample_rate, channels, bits, frames}` (claxon STREAMINFO). `check_compatible(&[(name, StemInfo)]) -> Result<Common {sample_rate, bits_out, frames}, String>` applies the rules above plus the memory cap; `bits_out` = 24 if any stem has more than 16 bits, else 16. `decode_i16(path, cancel: &AtomicBool, progress) -> StemPcm {channels, frames, samples: Vec<i16>}`. `StemReader`: streaming full-precision reader for the render (`read(&mut [f32] stereo, frames) -> frames`, silence after the end). |
| `src/mixer.rs` | `MIN_DB = -60.0` (= Off), `MAX_DB = 12.0`, `STEP_DB = 0.5`. `gain_factor(gain_db: Option<f32>, unmuted: bool) -> f32` (None or <= MIN_DB -> 0; muted -> 0; else `10^(dB/20)`). `clamp_db(f32) -> Option<f32>` snaps to the 0.5 dB grid and the range. `to_stereo_f32(stem, start_frame, frames, scratch)`: i16 -> f32 /32768, mono copied to both channels, zeros past the end. `mix(inputs: &[&[f32]], gains: &[f32], out: &mut [f32]) -> u32` sums interleaved-stereo blocks, hard-clips to [-1, 1] and returns the clipped-sample count. `quantise(x, bits) -> i32` rounds and clamps. Playback and Save both use **this `mix`**. |
| `src/transport.rs` | Pure state machine with injected `Instant`s: `play`, `pause`, `stop`, `seek(now)`, `tick(now)`, `ended()`. `RESUME_DELAY = 300 ms`. A seek while playing (or while a resume is pending) silences the output and sets `resume_at = now + 300 ms`; another seek inside the window moves `resume_at` again; `tick(now >= resume_at)` resumes. Pause and Stop cancel a pending resume. Exposes `audible()` and `shows_playing()` (= playing or resume pending, so the button stays "Pause" during a scrub). |
| `src/audio_out.rs` | `trait OutputBackend { fn name(); fn open(sample_rate, render: Box<dyn FnMut(&mut [f32]) + Send>) -> Result<Box<dyn OutputHandle>, String> }`. `render` fills interleaved stereo f32; dropping the handle closes the stream; `handle.error()` reports a device failure. Backends: **`CpalBackend`**: default device, the stems' rate, any sample format the device offers, extra device channels zeroed. The `cpal::Stream` is `!Send`, so it lives on its own thread until the handle is dropped. An unsupported rate gives "The audio output does not support <rate> Hz". **`NullBackend`**: a thread that calls `render` with 10 ms blocks paced by the wall clock. With `capture`, it also writes what it "played" to a float32 stereo WAV, with the header finalised on drop. **`ManualBackend`** (`#[cfg(test)]`): the test calls `pull(frames) -> Vec<f32>`. `backend_kind(e2e_hooks, env)` (pure): without the `e2e-hooks` feature it picks Cpal; **with `e2e-hooks` it always picks Null**, and `CALLIOPE_E2E_AUDIO=capture:<abs path>` adds the capture. Logs `calliope: audio backend=<cpal\|null\|capture> rate=<hz>` when a stream opens. |
| `src/editor.rs` | **`EditorManager`** (Tauri-managed behind a `Mutex`) holds the current `Session`, the backend, the per-track **remembered mixes** for this app run, and the running save job. **`Session`** holds id, title, track dir, stem entries, the target backing variant id, an `Arc<PlayerShared>`, a `Transport`, `solo: Option<usize>`, the output handle, the event sink and the last emit time. `PlayerShared` = decoded stems; per-stem `AtomicU32` effective gains (f32 bits, solo already applied); an `AtomicU64` position in frames; an `AtomicBool` audible; an `AtomicU64` clipped counter. **Commands:** `open`, `close`, `play` (ends solo), `lane_play(name)`, `end_solo`, `pause`, `stop`, `seek(ms)`, `nudge(delta_ms)`, `set_stem(name, gain_db, unmuted)`, `snapshot()`, `tick(now)`. **Render callback:** reads the position and mixes in 512-frame chunks (pre-allocated scratch, no allocation or locks in the callback). It then does `compare_exchange(old, old + n)` on the position, so a concurrent seek (plain `store`) always wins. Past the end it outputs silence and sets an `ended` flag. **Ticker:** a thread spawned in `gui.rs` calls `tick(Instant::now())` every 20 ms. It applies a due resume, turns `ended` into stop + position 0, updates `clipping`, reports device errors, and emits `transport` events: immediately on a change, else every 100 ms while `shows_playing`. **Output stream:** opens on the first Play of a session, stays open (silent) while paused, and closes on Stop, on session close and at app exit. |
| `src/backing_render.rs` | `render_flac(input: RenderInput, part: &Path, cancel, progress) -> Result<RenderStats {clipped_samples}, String>`. It opens a `StemReader` per **checked** stem, mixes 4096-frame blocks with `mixer::mix` and the saved gains, quantises to `bits_out`, and feeds `flacenc` through a custom `Source` (progress = frames read / total, at most 10 events/s). It writes the stream to `part` (`create_new`, fsync). The encoded stream is held in memory before writing (15 min of stereo is about 100-160 MB). Also the save **job**: one at a time, its own std thread, a cancel flag, events over the editor sink. |
| `src/track_meta.rs` | New field `backings: Vec<BackingVariant>` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`, so tracks without backings are written exactly as before). `BackingVariant {id, name, file, created, modified, sample_rate, bits, mix: serde_json::Value}`. Read validation (safety): `id` matches the stem-name rule `^[a-z0-9][a-z0-9_-]{0,31}$` and is unique; `file` = `backings/<plain file name>`, unique among all of the track's files (the existing `claim` check); `name` is text of 1..=100 characters. `mix` is interpreted only by the editor, leniently (unknown stems ignored, bad values defaulted), so it never makes a track a "problem". The track's `missing` list includes backing files. |
| `src/repository.rs` | `save_backing(BackingSave { id, variant: Option<&str>, stems: &[StemEntry], part: &Path, mix: Value, sample_rate, bits }) -> Result<SaveResult, String>` (2.6). `FileTrash`: `TablatureTrash` generalised with a folder suffix (`tablatures` \| `backing`), behaviour unchanged. |
| `src/ipc.rs`, `src/gui.rs`, `build.rs`, `capabilities/main.json` | Commands of 2.8. `EditorManager` is managed in `setup`; the ticker thread starts there. `RunEvent::Exit` stops playback and cancels a save (2 s cap, as for imports). |

### 2.5 Data: `track.json` with backing variants

The track **keeps `"type": "stem"`**, its stems, and `"audio": null`. Backings are a list, ready for several named
variants (the owner's plan). This feature creates and replaces only one variant, with the default id `backing` and the
name "Backing":

```json
  "audio": null,
  "backings": [
    {
      "id": "backing",
      "name": "Backing",
      "file": "backings/backing.flac",
      "created": "2026-10-08T12:00:00Z",
      "modified": "2026-10-08T12:30:00Z",
      "sample_rate": 44100,
      "bits": 16,
      "mix": {
        "stems": [
          {"name": "vocals", "gain_db": -3.5, "unmuted": true},
          {"name": "guitar", "gain_db": 0.0, "unmuted": false},
          {"name": "bass", "gain_db": null, "unmuted": true}
        ]
      }
    }
  ],
```

- **Why not `audio: "backing.flac"`**:
  - `audio` is one file, but a stem track will have several variants.
  - Pointing `audio` at one of them would duplicate the list and could disagree with it.
  - `audio` keeps its meaning: the single audio file of a plain (imported) backing track.

  The Player chooses among `backings` for stem tracks; the first entry is the default until a later spec says
  otherwise.
- **Files**: `backings/<id>.flac`. The variant `id` is a stable slug, fixed when the variant is created; `name` is the
  display name, editable later without renaming files. Future variants get ids from their names (e.g. `no-guitar`).
  Ids and file names stay unique, and a clash gets a suffix: `-2`, `-3`, ...
- `gain_db` is a number on the 0.5 dB grid in -59.5..=12, or `null` = Off. `unmuted` is the lane checkbox. All stems
  are listed, unchecked ones too, so reopening restores the whole mixer. The solo is never stored.
- No schema bump: `backings` is additive. Older v2 builds keep it as an unknown field and write it back unchanged,
  and their Library still deletes and exports the whole folder.
- **Initial mixer state** when a track is opened, in this order:
  1. the remembered mix from this app run (the Editor was used on the track before; it survives switching tracks);
  2. else the mix of the first entry in `backings` (matched by stem name; unknown names ignored, missing ones
     default, values snapped and clamped);
  3. else the default: **all stems unchecked at 0 dB**, so Mix Play stays disabled until a stem is checked (spec).

### 2.6 Save transaction (follows the repository's existing rules)

1. The save job renders into `<track>/.calliope-backing-<uuid>.part` (unique, `create_new`; a dot name, so scans ignore
   it). Cancel or failure removes it, since it is provably ours. A crash may leave it behind (harmless; listed in the
   architecture's accepted limits).
2. Under the repository mutex, re-read `track.json`. The track must still exist, be `type: stem`, and list **the same
   stems** (names and files) as the editor session. Otherwise the save fails with
   `conflict: the stems of this track changed on disk; select it again`.
   - Other metadata edits made meanwhile (e.g. a Library title edit) are kept, because only `backings` and `modified`
     change.
   - A whole-file revision check would make every Library edit block the Editor's Save, so it isn't used.
3. **Target variant**: the session's variant id. That is the first entry of `backings` when the track was opened, else
   `backing`.
   - **If the variant exists:** its file is replaced.
   - **If it doesn't:** it is created. If `backings/backing.flac` is taken by a file that isn't listed (a user's
     file), or `backing` is already used as an id, the id and file become `backing-2`, `backing-3`, ...

   Calliope never overwrites a file it doesn't own. Create `backings/` if needed (never through a symlink).
4. **Replace** (same file name): move the old file to `trash/<stamp>-<id>-backing/`, then `hard_link` the part to the
   name (no clobber). **Create**: link first. This is the same order, with the same accepted crash window, as the
   same-name tablature replace.
5. `write_atomic` the metadata: the variant's `file`, `sample_rate`, `bits`, `mix` and `modified` = now; `created` only
   on creation; the track's `modified` = now. Then remove the part and return `SaveResult` (track + warnings).

**Output format** (FLAC confirmed by the owner):
- the stems' sample rate, stereo;
- `bits_out` bits (16, or 24 if a stem is 24-bit);
- length = longest stem;
- sum of checked stems x gain, hard-clipped at full scale, rounded, no dither.

Save is enabled when at least one stem is checked (spec). Playback may continue during the render.

### 2.7 Behaviour (spec gaps filled; owner decisions marked)

- **Editor pane active** = the selected track has `stems.length > 0` and none of its stem files is in `missing`.
  - **Inactive** (no selection, or a track without stems): lanes are not shown and the Mix lane is shown disabled,
    with "Select a track with stems to build its backing track." (or "<title> has no stems.").
  - **Missing stem files:** inactive plus "Stem file stems/drums.flac is missing."
  - **Load errors:** shown inline (role=alert), never as pop-ups.
- **Lanes**: one per entry of `stems`, in `track.json` order. The 6 stems of `htdemucs_6s` give 6 lanes, which settles
  the spec's "5" by its own rule "as many lanes as the track has".
  - Label = stem name with a capital first letter.
  - The lane area scrolls; the Mix lane stays pinned below it.
  - **`original.flac` is hidden in the Editor** (owner; the Player will use it later).
- **Mix Play/Pause**: enabled when at least one stem is checked, or while playing (so you can always pause). Label
  "Play" / "Pause" from `shows_playing`. Unchecking every lane while playing keeps playing (silently).
- **Stop**: enabled while playing or when the position is > 0. It stops and returns to `0:00.0`. The end of the track
  acts as Stop.
- **Time format (owner)**: `M:SS.T`, e.g. `3:07.4` (minutes unpadded, seconds 2 digits, tenths 1 digit; display =
  floor to the tenth).
  - **Input** accepts `M:SS.T`, `M:SS:T` (the spec's original wording, harmless), `M:SS`, and plain seconds `S` or
    `S.T`.
  - **Commit:** Enter or blur. A value outside 0..duration, or one that doesn't parse, marks the field invalid
    (`aria-invalid`, message "Enter a time between 0:00.0 and 3:07.4") and doesn't seek. Escape reverts.
  - While the field is focused and edited, position updates don't overwrite it.
- **Wheel over the time field**: each wheel event with `deltaY < 0` (wheel up) = -0.1 s, `deltaY > 0` = +0.1 s, as the
  spec says; `preventDefault` stops the page scrolling. Rust computes it from the true current position
  (`editor_nudge`), clamped to 0..duration. It works whether playing or not.
- **Any position change while playing** (slider drag, wheel, typed time) follows the 0.3 s rule: silence now, resume
  0.3 s after the last change. The frontend coalesces slider and volume IPC (at most one call in flight, the latest
  value wins), so dragging doesn't queue hundreds of calls.
- **Playback stops when the selected track changes or the view changes (owner).**
  - **Track change:** stop, close the session (its mix is remembered for this run), open the new track. A load still
    running for the previous selection is cancelled.
  - **Leaving the Editor view:** `editor_stop`. The session stays open and the mix is kept; coming back shows the same
    track stopped at `0:00.0`. Implemented in the frontend's view switch (`App.svelte`/`views` handling) by calling
    `leaveEditor()`; Rust doesn't know about views.
- **Re-saving** replaces the variant's file (the old one goes to the repository trash). There is no confirmation
  dialog: nothing is lost, and the UI guide says no pop-ups. The pane shows the target: "Save as: Backing
  (backings/backing.flac)". There is no naming or multi-variant UI in this feature.
- **Library pane**: for tracks with backings, a "Backing tracks" row lists each variant name with its file (read-only),
  next to the existing Stems row.
- **Keyboard** (hands on the guitar):
  - In the Editor view, `Space` = Mix Play/Pause, unless the focus is in a text field, checkbox, slider, tree item or
    button (those keep their own Space behaviour). `Ctrl+S` = Save.
  - Tab order: tree, lanes (Play, Unmute, Volume), then the Mix lane left to right.
  - Large controls: buttons at least ~50 px tall, 28 px checkboxes, slider thumbs at least 24 px.
- **Tree**: the Library's tree column (toolbar, tree, hints, problems) is extracted into a shared
  `TrackBrowser.svelte`.
  - Both views use it with the **same state** (`lib` in `library-state.svelte.ts`), so a track selected in one view
    is selected in the other.
  - If the Library is in edit mode, the Editor's tree is locked with the same hint.
  - The Editor rescans on mount like the Library (unless in edit mode), and after a Save.
- **Tabs**: the right pane keeps the Editor tab strip. The mixer is the "Stems" tab, and the "Assembly" placeholder
  tab is removed (this feature is the assembly). BPM & sections, Tablature and MIDI cues stay placeholders.

### 2.8 IPC (pattern of `docs/architecture.md`)

Rust types live in `src/editor.rs`, TS mirrors in `src/ui/lib/ipc.ts`. Every editor command takes the track `id` and
fails with "no editor session for <id>" when it isn't the open one, which protects against stale UI calls.

```ts
interface LaneState { name: string; gain_db: number | null /* null = Off */; unmuted: boolean }
interface TransportState {
  playing: boolean /* shows_playing */; position_ms: number; resume_pending: boolean;
  solo: string | null; clipping: boolean
}
interface BackingVariant { id: string; name: string; file: string; created: string; modified: string;
  sample_rate: number; bits: number; mix: unknown }
interface EditorSnapshot {
  id: string; title: string; stems: LaneState[]; duration_ms: number; sample_rate: number;
  transport: TransportState; variant: { id: string; name: string; file: string; exists: boolean };
  saving: number | null /* 0..1 */
}
type EditorEvent =                       // tagged by "kind"
  | { kind: 'loading'; id: string; done: number; total: number }
  | ({ kind: 'transport'; id: string } & TransportState)
  | { kind: 'saving'; id: string; progress: number }
  | { kind: 'saved'; id: string; file: string; clipped_samples: number; track: TrackRecord; warnings: string[] }
  | { kind: 'save-failed'; id: string; message: string }
  | { kind: 'save-cancelled'; id: string }
  | { kind: 'audio-error'; id: string; message: string };   // playback stopped
```

| Command | Returns | Notes |
|---|---|---|
| `open_editor(id, events: Channel<EditorEvent>)` | `EditorSnapshot` | async + spawn_blocking; closes the previous session; decodes with `loading` events; a superseded load fails with `superseded` (ignored by the UI) |
| `close_editor()` | `()` | stops playback, remembers the mix |
| `get_editor()` / `watch_editor(events)` | `EditorSnapshot \| null` | re-attach after a webview reload (existing pattern) |
| `editor_play(id)` | `TransportState` | Mix Play: ends solo; fails with "Check a stem first" when no stem is checked |
| `editor_lane_play(id, name)` | `TransportState` | checks the stem, solos it, plays (2.3); a second call on the soloed stem ends the solo |
| `editor_end_solo(id)` | `TransportState` | |
| `editor_pause(id)`, `editor_stop(id)` | `TransportState` | Stop ends solo |
| `editor_seek(id, position)` (ms), `editor_nudge(id, delta)` (ms) | `TransportState` | clamped; 0.3 s rule while playing |
| `editor_set_stem(id, name, gain, unmuted)` | `LaneState` | `gain` in dB or null (Off), snapped to 0.5 dB and clamped; unchecking the soloed stem ends solo |
| `save_backing(id)` | `EditorSnapshot` | starts the save job with the checked stems' mix; progress on the session channel; one save at a time |
| `cancel_backing_save(id)` | `()` | |

Each new command goes into `generate_handler!`, `COMMANDS` in `build.rs` and `capabilities/main.json` (static test).
`TrackRecord` gains `backings: BackingVariant[]`.

**Log lines** (for GUI tests).
- Frontend (`calliope-ui: `):
  - `editor active id=<id> stems=<n>`
  - `editor inactive id=<id|none>`
  - `editor play|pause|stop position=<ms>`
  - `editor solo name=<n|none>`
  - `editor leave stop`
  - `editor seek position=<ms>`
  - `editor nudge delta=<ms>`
  - `editor stem name=<n> gain=<db|off> unmuted=<b>`
  - `editor saved id=<id> file=<f> clipped=<n>`
  - `error editor <command>: <msg>`
- Rust (`calliope: `):
  - `audio backend=<b> rate=<hz>`
  - `editor open id=<id> stems=<n> duration_ms=<ms>`
  - `editor transport playing=<b> audible=<b> position_ms=<ms> solo=<n|none> clipping=<b>` (on changes only, not on
    100 ms ticks)
  - `editor saved id=<id> file=<f> bits=<b> clipped=<n>`

### 2.9 Frontend modules

| File | Content |
|---|---|
| `src/ui/lib/time-format.ts` | `formatPosition(ms)`, `parsePosition(text) -> number \| null` (ms), pure |
| `src/ui/lib/gain-format.ts` | `formatGain(db: number \| null) -> string` (`Off`, `0.0 dB`, `+6.5 dB`, `-12.0 dB`), `sliderToGain(v) / gainToSlider(db)` (slider value -60 = Off), pure |
| `src/ui/lib/editor-state.svelte.ts` | Module `$state` `ed`: `status: 'inactive'\|'loading'\|'active'\|'error'`, `trackId`, `message`, `loading`, `stems`, `durationMs`, `transport`, `variant`, `saving`, `saveMessage`, `saveError`. Functions: `syncSelection(track)`, `leaveEditor()`. Derived: `canPlay()`, `stopEnabled()`, `saveEnabled()`. Actions: `togglePlay`, `stop`, `lanePlay`, `setUnmuted`, `setGain` (coalesced), `resetGain`, `seek` (coalesced), `nudge`, `save`, `cancelSave`. Plus the event handler and a stale-id guard. |
| `src/ui/lib/components/ui/slider/`, `.../checkbox/` | shadcn-style wrappers over bits-ui `Slider` and `Checkbox` (written in our tree like the existing ones), large sizes; the slider supports a tick mark |
| `src/ui/components/library/TrackBrowser.svelte` | tree column extracted from `LibraryView.svelte` (heading passed in) |
| `src/ui/components/editor/StemLane.svelte`, `MixLane.svelte`, `TimeField.svelte`, `EditorPane.svelte` | The lanes; the time field (edit + wheel); the pane (inactive / loading / error / active, solo display, CLIP badge, save target, progress and messages) |
| `src/ui/views/EditorView.svelte` | `TrackBrowser` on the left; tabs + `EditorPane` on the right; Space and Ctrl+S |

## 3. Tasks

Every task ends with the headless suite green: `cd src/calliope-gui && npm test`. GUI tasks also run
`DISPLAY=:1 npm run test:gui`.

### Task 1: Editor test fixtures
- routine: yes
- files: `src/calliope-gui/tests/fixtures/editor/make-fixtures.sh` (new),
  `src/calliope-gui/tests/fixtures/library-editor/**` (generated, committed),
  `src/calliope-gui/tests/fixtures/import/README.md` (one line pointing to the new script)
- does: a bash script in the style of `tests/fixtures/import/make-fixtures.sh` (the same `ff()` helper with
  `-fflags +bitexact`, idempotent, `FORCE=1` regenerates). It builds the repository `tests/fixtures/library-editor/`
  with `calliope-repository.json` = `{"schema_version": 1}` and the tracks below.
  - **Common `track.json` fields:**
    - `schema_version` 2, `band` "The Example Band", `album` "Editor Tests", `composers` [];
    - `year`, `source_url`, `copyright`, `original` all null;
    - `stem_model` "htdemucs_6s" for stem tracks, else null;
    - `tablatures` [];
    - `imported` and `modified` "2026-10-07T10:00:00Z";
    - `stems` as listed, with `file` = `stems/<name>.flac`.
  - **Stem source command:** all stems are 22050 Hz s16 FLAC from
    `-f lavfi -i "sine=frequency=F:sample_rate=22050:duration=D" -sample_fmt s16 -c:a flac`.
  - `0199c0a0-0000-7000-8000-000000000201` "Four Lanes": type stem, `audio` null.
    - vocals F=440 D=12 `-ac 1`
    - drums F=220 D=12 `-ac 1`
    - bass F=110 D=12 `-ac 2`
    - guitar F=660 D=10 `-ac 1`
  - `...0202` "Mixed Before": type stem, `audio` null.
    - Stems: vocals (F=440 D=6 `-ac 1`) and guitar (F=660 D=6 `-ac 1`).
    - File `backings/backing.flac`: F=330 D=6 `-ac 2`, 22050 Hz s16.
    - `track.json` gets:
      `"backings": [{"id": "backing", "name": "Backing", "file": "backings/backing.flac", "created": "2026-10-07T10:00:00Z", "modified": "2026-10-07T10:00:00Z", "sample_rate": 22050, "bits": 16, "mix": {"stems": [{"name": "vocals", "gain_db": -6.0, "unmuted": true}, {"name": "guitar", "gain_db": 0.0, "unmuted": false}]}}]`.
  - `...0203` "Plain Backing": type backing, `audio` "backing.mp3" = a copy of `../import/tagged.mp3`, `stems` [].
  - `...0204` "Six Lanes": type stem, the six files of `../import/stems/` copied, in this order: vocals, drums, bass,
    guitar, piano, other.
  - `...0205` "Missing Stem": type stem, stems vocals (a copy of `../import/stems/vocals.flac`) and drums (listed,
    file deliberately absent).
- done when:
  - `bash tests/fixtures/editor/make-fixtures.sh` succeeds twice in a row, and the second run changes no file
    (`git status --porcelain` is clean after committing the first run);
  - `ffprobe` reports 22050 Hz and the listed channel counts and durations for the "Four Lanes" stems;
  - every `track.json` parses (`python3 -m json.tool`);
  - `npm test` passes.

### Task 2: Audio dependencies and licence records
- routine: yes
- files: `src/calliope-gui/Cargo.toml`, `Cargo.toml` (workspace root), `docs/licences.md`, `src/calliope-gui/README.md`
- does:
  - Add `claxon = "0.4"`, `flacenc = { version = "0.5", default-features = false }` and `cpal = "0.18"` to
    `[dependencies]` of calliope-gui.
  - In the root `Cargo.toml`, add `[profile.dev.package.claxon]` and `[profile.dev.package.flacenc]` with
    `opt-level = 3`.
  - Add the three rows to the calliope-gui crate table of `docs/licences.md`. Read the licence and locked version from
    `cargo metadata` (all Apache-2.0). Purposes: "FLAC decoding of stems", "FLAC encoding of backing tracks", "audio
    output (ALSA on Linux)".
  - Also in `docs/licences.md`, add a sentence that `cpal` links the system `libasound` (alsa-lib, LGPL-2.1+, a system
    library, dynamically linked, not bundled), and one that `symphonia` (MPL-2.0) is rejected by the owner.
  - In the README's prerequisites, add `alsa-lib` (the Arch package name).
- done when:
  - `cargo build -p calliope-gui` succeeds;
  - `cargo test -p calliope-gui --test frontend` passes (licence record test);
  - `npm test` passes.

### Task 3: Stem decoding (`stem_audio.rs`)
- routine: no
- files: `src/stem_audio.rs` (new), `src/main.rs` (mod)
- does:
  - `probe`.
  - `check_compatible`: same rate, 1-2 channels, 8..=24 bits; a decoded-size cap of 1.5 GiB, computed as
    sum(frames x channels x 2 bytes); `bits_out`; `frames` = max.
  - `decode_i16`: rounding for >16-bit; the cancel flag is checked per block; progress callback.
  - `StemReader`: full-precision streaming to f32 stereo, mono duplicated, zero-fill after the end.
  - Error texts name the stem.
- done when: the module tests pass. Their FLAC inputs are generated in a temp dir with `flacenc` from known sample
  sequences. They check:
  - exact i16 samples for 16-bit mono and stereo;
  - 24-bit rounding;
  - mismatched rates, 3 channels and the size cap are rejected with the stem named;
  - `StemReader` returns the exact samples, then zeros;
  - a set cancel flag stops decoding with an error;
  - a missing or non-FLAC file gives an error naming it.

### Task 4: Mixer (`mixer.rs`)
- routine: no
- files: `src/mixer.rs` (new), `src/main.rs`
- does: the constants, `gain_factor`, `clamp_db`, `to_stereo_f32`, `mix` (returns the clipped count) and `quantise`, as
  in 2.4. No allocation in `mix`/`to_stereo_f32`.
- done when: the module tests pass:
  - `gain_factor(Some(0.0), true) == 1.0`;
  - `gain_factor(Some(-6.0), true)` within 1e-6 of 0.501187;
  - `gain_factor(Some(12.0), true)` within 1e-5 of 3.981072;
  - `None`, `Some(-60.0)` and `unmuted=false` give 0;
  - `clamp_db(13.0) == Some(12.0)`, `clamp_db(-0.3) == Some(-0.5)` (snap to the grid, round half away from zero),
    `clamp_db(-80.0) == None`;
  - two stems sum;
  - mono is copied to both channels;
  - frames past a stem's end are zero;
  - a +12 dB stem at 0.5 amplitude clips to 1.0 and `mix` returns the exact number of clipped samples (0 when nothing
    clips);
  - `quantise(1.0, 16) == 32767`, `quantise(-1.0, 16) == -32768`, `quantise(0.5, 16) == 16384`, plus the 24-bit
    equivalents.

### Task 5: Transport state machine (`transport.rs`)
- routine: no
- files: `src/transport.rs` (new), `src/main.rs`
- does: the state machine of 2.4, with injected `Instant`s. The position stays in the player, not here.
- done when: the module tests pass:
  - play -> audible;
  - pause -> not audible, `shows_playing` false;
  - seek while playing at t0 -> not audible, `shows_playing` true; `tick(t0+299ms)` still silent;
    `tick(t0+300ms)` audible;
  - seeks at t0 and t0+200ms -> resume at t0+500ms, not before;
  - seek while paused -> stays paused, no resume;
  - pause during the window -> no resume;
  - stop clears everything.

### Task 6: Audio output backends (`audio_out.rs`)
- routine: no
- files: `src/audio_out.rs` (new), `src/main.rs`, `tests/frontend.rs` (static check)
- does:
  - the trait plus `CpalBackend`, `NullBackend` (paced 10 ms blocks; optional float32 WAV capture) and
    `ManualBackend` (cfg(test));
  - `backend_kind` and `select_backend()`;
  - the `audio backend=` log line;
  - the cpal stream lives on its own thread, and device errors are stored for `handle.error()`.
- done when:
  - the module tests pass:
    - `ManualBackend::pull(n)` returns what `render` produced;
    - `NullBackend` calls render at roughly real time (100 ms of wall time -> 2205 +/- 30 % frames at 22.05 kHz) and
      stops after the handle is dropped;
    - its capture WAV has a valid header (RIFF/WAVE, format 3, 2 channels, the rate) and the rendered samples;
    - `backend_kind` is tested for every case: with `e2e_hooks = true` it never yields Cpal, whatever the env says,
      and `capture:` needs an absolute path, else plain Null;
  - a static test in `tests/frontend.rs` checks that `CpalBackend` appears only in `src/audio_out.rs` and
    `src/gui.rs`;
  - `cargo clippy --workspace --all-targets -- -D warnings` is clean.

### Task 7: Editor sessions and playback engine (`editor.rs`)
- routine: no
- files: `src/editor.rs` (new), `src/editor_tests.rs` (new, included with `#[cfg(test)] #[path]` like
  `repository_tests.rs`), `src/main.rs`
- does: `EditorManager`/`Session` as in 2.4:
  - open with progress, cancel and supersede (generation counter);
  - the initial mix (remembered > first backing variant > all unchecked at 0 dB) and the target variant;
  - the render callback (chunked mix, CAS position, ended flag, clipped counter);
  - `play` (ends solo; fails without a checked stem), `lane_play`, `end_solo`, `pause`, `stop` (position 0, solo off,
    output closed);
  - `seek`/`nudge` (clamped, transport 0.3 s rule) and `set_stem` (snaps the gain; unchecking the soloed stem ends
    solo);
  - `tick(now)` (resume, end, `clipping` with a 1 s hold, device error, event throttling), `close` (remembers the mix),
    and the snapshot.

  Events go to the sink; there are no Tauri types.
- done when: the tests pass, using `ManualBackend` and fixture copies of `library-editor`:
  - **Open:**
    - "Four Lanes" gives 4 lanes, duration 12000 ms, 22050 Hz, all unchecked at 0 dB, and `editor_play` fails with
      "Check a stem first";
    - "Mixed Before" restores vocals -6 dB checked and guitar unchecked, with target variant `backing` (exists);
    - "Missing Stem" fails, naming `stems/drums.flac`.
  - **Playback:**
    - after checking two stems and `play`, the pulled audio equals the expected mix of the checked stems (computed in
      the test from the decoded samples and `gain_factor`);
    - a gain change applies from the next pulled block.
  - **Solo:**
    - `lane_play("guitar")` while stopped checks guitar (the snapshot shows `unmuted: true`), starts playback, and the
      pulled audio contains only guitar;
    - the other lanes' `unmuted` values are unchanged;
    - `lane_play` on another stem moves the solo;
    - `lane_play` on the soloed stem, `end_solo`, Mix `play`, `stop` and unchecking the soloed stem each end the solo;
    - pause keeps it.
  - **Clipping:** two stems at +12 dB raise `clipping: true` in the next transport event; it returns to false 1 s
    (injected time) after the last clipped block.
  - **Pause:** pulled blocks are zero and the position doesn't move.
  - **Seek and nudge:**
    - `seek` while playing silences, and `tick(+300ms)` resumes at the new position;
    - `nudge(-100)` at 0 stays 0, and at the end stays at the end.
  - **End and events:**
    - playing past the end emits `transport {playing: false, position_ms: 0}`;
    - transport events while playing come at most once per 100 ms.
  - **Sessions:**
    - a second `open` cancels a load in progress (`superseded`);
    - `close` then `open` of the same track restores the remembered mix.

### Task 8: `backings` metadata and repository `save_backing`
- routine: no
- files: `src/track_meta.rs`, `src/repository.rs`, `src/repository_tests.rs`, `src/fsutil.rs` (`FileTrash`)
- does:
  - the `backings` field and its read validation (2.4, 2.5), including backing files in `missing`;
  - the `save_backing` transaction (2.6), including variant/id/file choice, the `backings/` folder and trash handling;
  - `TablatureTrash` becomes `FileTrash` with a suffix; existing callers keep their behaviour.
- done when: the track_meta and repository tests pass, on temp copies of `library-editor`:
  - **Read validation:**
    - a bad variant id, a `file` outside `backings/`, a duplicate id, or a file clashing with a stem/tablature makes
      the track a problem;
    - a malformed `mix` does not.
  - **First save on "Four Lanes":**
    - creates `backings/backing.flac` and one variant (`id` backing, `name` Backing, `created` = `modified` = now,
      `sample_rate`, `bits`, `mix`);
    - keeps `type: stem`, `audio: null`, the stems and every other field;
    - updates the track's `modified`.
  - **Second save:** moves the previous file to `trash/<stamp>-<id>-backing/`, puts the new one in place, keeps
    `created` and updates the variant's `modified`.
  - **No clobber:** an unlisted user file `backings/backing.flac` makes the save create variant `backing-2` with
    `backings/backing-2.flac`, and the user file stays byte-identical.
  - **Conflict:** a changed stems list in `track.json` gives `conflict:` and leaves all files untouched.
  - **Merging:** a title edited on disk meanwhile is kept.
  - **No change for other tracks:** a track without backings saved through the Library is byte-identical to before
    this change (no `"backings": []`).
  - **Safety:** a symlinked `backings/` is refused; the sibling "outside" folder is unchanged (existing helper).
  - **Regressions:** all existing repository and track_meta tests still pass.

### Task 9: Offline render and the save job (`backing_render.rs`)
- routine: no
- files: `src/backing_render.rs` (new), `src/editor.rs` (save job wiring), `src/editor_tests.rs`, `src/main.rs`
- does:
  - `render_flac` (2.4);
  - the save job: part file, render, `Repository::save_backing`, the events `saving` / `saved` (with
    `clipped_samples`) / `save-failed` / `save-cancelled`;
  - cancel, the part file removed on every failure, one job at a time;
  - solo is ignored.
- done when: the tests pass:
  - **Exact samples:** decoding the saved file with claxon gives exactly
    `quantise(clip(sum gain_k x s_k))` per sample. The case uses 16-bit fixture stems, gains 0 dB / -6 dB / +6 dB, one
    stem unchecked, mono+stereo, and a shorter stem padded with silence.
  - **Format:** 22050 Hz, stereo, 16 bits, length = longest stem.
  - **24-bit:** with a 24-bit test stem the output is 24-bit.
  - **Clipping:** a +12 dB mix reports the same `clipped_samples` as the sum of `mix` return values over the same
    blocks, and the clipped samples are at full scale.
  - **Solo:** saving during a solo saves the checked stems, not the solo.
  - **Failures:**
    - a cancel mid-render leaves no part file and `track.json` unchanged;
    - a forced repository conflict gives `save-failed` and no part file.
  - **Progress:** events are monotonic, end at 1.0, and come at most 10/s.
  - **Refusal:** a save with no checked stem is refused.

### Task 10: IPC commands and TS wrappers
- routine: no
- files: `src/ipc.rs`, `src/gui.rs`, `build.rs`, `capabilities/main.json`, `src/ui/lib/ipc.ts`,
  `src/ui/lib/ipc-editor.test.ts` (new), `src/ui/lib/fixture-tracks.ts` (editor fixture records for vitest)
- does:
  - the commands of 2.8 as thin async wrappers (blocking work in `spawn_blocking`);
  - `EditorManager` managed with `select_backend()`, the ticker thread and the Exit hook;
  - TS types and wrappers (`openEditor(id, onEvent)`, etc., with a Channel like `channel()` for imports);
  - `TrackRecord.backings`.
- done when:
  - the static command-list test in `tests/frontend.rs` passes;
  - vitest `ipc-editor.test.ts` checks each wrapper's command name and argument names with `mockIPC`;
  - `npm test` passes.

### Task 11: Time formatting and parsing
- routine: yes
- files: `src/ui/lib/time-format.ts` (new), `src/ui/lib/time-format.test.ts` (new)
- does:
  - `formatPosition(ms: number): string` = `${m}:${ss}.${t}`, with:
    - `m = floor(ms/60000)`;
    - `ss` = two-digit `floor(ms/1000) % 60`;
    - `t = floor(ms/100) % 10`;
    - negative input treated as 0.
  - `parsePosition(text: string): number | null`. Trim, then:
    - accept `^(\d+):([0-5]\d)(?:[.:](\d))?$` -> `(m*60 + s)*1000 + t*100`;
    - or `^(\d+)(?:\.(\d))?$` (plain seconds) -> `s*1000 + t*100`;
    - anything else -> `null`.

    No range checks here.
- done when: `npx vitest run src/ui/lib/time-format.test.ts` passes with at least these cases, and `npm run check`
  passes:
  - format: 0 -> `0:00.0`, 99 -> `0:00.0`, 100 -> `0:00.1`, 7400 -> `0:07.4`, 187400 -> `3:07.4`,
    600000 -> `10:00.0`, -5 -> `0:00.0`;
  - parse: `3:07.4` -> 187400, `3:07:4` -> 187400, `3:07` -> 187000, `0:00.0` -> 0, ` 1:02.3 ` -> 62300,
    `75` -> 75000, `75.5` -> 75500;
  - null for: `3:7.4`, `3:60.0`, `3:07.45`, `abc`, `` (empty), `-1`, `1:02.`.

### Task 12: Gain formatting
- routine: yes
- files: `src/ui/lib/gain-format.ts` (new), `src/ui/lib/gain-format.test.ts` (new)
- does:
  - `formatGain(db: number | null): string`:
    - `null` or `<= -60` -> `Off`;
    - `0` -> `0.0 dB`;
    - positive -> `+` + one decimal + ` dB`;
    - negative -> `-` + one decimal + ` dB` (ASCII hyphen-minus).
  - `sliderToGain(v: number): number | null` -> `null` when `v <= -60`, else `v`.
  - `gainToSlider(db: number | null): number` -> `-60` for `null`, else `db` clamped to [-60, 12].
- done when: `npx vitest run src/ui/lib/gain-format.test.ts` passes with at least these cases, and `npm run check`
  passes:
  - format: `null` -> `Off`, -60 -> `Off`, 0 -> `0.0 dB`, 6.5 -> `+6.5 dB`, -12 -> `-12.0 dB`, 12 -> `+12.0 dB`,
    -0.5 -> `-0.5 dB`;
  - `sliderToGain(-60)` -> null, `sliderToGain(-59.5)` -> -59.5;
  - `gainToSlider(null)` -> -60, `gainToSlider(20)` -> 12.

### Task 13: Shared tree column, Editor layout, Library backings row
- routine: no
- files: `src/ui/components/library/TrackBrowser.svelte` (new), `src/ui/views/LibraryView.svelte`,
  `src/ui/components/library/TrackPane.svelte`, `src/ui/views/EditorView.svelte`, `src/ui/lib/views.ts`,
  `src/ui/lib/views.test.ts`, `src/ui/App.test.ts` and any test expecting the "Assembly" tab or the old Editor
  placeholder, `tests/gui_e2e.rs` if it checks the Editor tabs
- does:
  - Extract the tree column (toolbar, hints, alerts, tree, problems; heading passed as a prop/snippet; Ctrl+F stays
    per view) and use it in both views.
  - Editor layout: `TrackBrowser` on the left (same width as the Library); tabs on the right, with the "Stems" tab
    hosting an `EditorPane` placeholder slot.
  - Remove the "Assembly" tab; use the editor feature name `gui-backing-track-editor` in `VIEWS`/`TRACK_TABS`.
  - The Editor's `onMount` rescans like the Library.
  - Library `TrackPane`: a read-only "Backing tracks" row listing `name (file)` per variant when `backings` is not
    empty.
- done when:
  - all existing Library vitest and GUI library tests pass, with unchanged behaviour;
  - a vitest shows the Editor renders the same tree, and that selecting a track in the Editor tree selects it in the
    Library state;
  - a TrackPane vitest shows "Backing (backings/backing.flac)" for the "Mixed Before" record and no row for a track
    without backings;
  - `npm test` passes.

### Task 14: Slider and checkbox components
- routine: no
- files: `src/ui/lib/components/ui/slider/{slider.svelte,index.ts}`, `src/ui/lib/components/ui/checkbox/{checkbox.svelte,index.ts}`,
  `src/ui/lib/components/ui/slider/slider.test.ts`
- does:
  - Slider: a wrapper over bits-ui `Slider` with a single value, `aria-label` and `aria-valuetext` props, a large thumb
    (>= 24 px), an amber range, a visible focus ring, and an optional `tick` value drawn as a mark (0 dB).
  - Checkbox: a wrapper over bits-ui `Checkbox`, 28 px, amber when checked, with a focus ring.
  - Both CSP-safe (no static `style=`) and in both themes.
- done when:
  - vitest renders both: keyboard arrows change the slider value by one step, Space toggles the checkbox, and the tick
    is rendered at the right position;
  - the static CSP/style tests pass;
  - `npm test` passes.

### Task 15: Editor frontend state
- routine: no
- files: `src/ui/lib/editor-state.svelte.ts` (new), `src/ui/lib/editor-state.test.ts` (new), `src/ui/App.svelte`
  (call `leaveEditor()` when the view changes away from `editor`)
- does: as in 2.9, following the rules of 2.3 and 2.7: the activation rule, solo, enables, coalescing, stale responses
  ignored, `superseded` ignored, events applied, a library reload after `saved`, stop on leaving the view, and the log
  lines of 2.8.
- done when: the vitest tests with `mockIPC` pass:
  - **Activation:**
    - a track without stems -> `inactive`, no `open_editor` call, `editor inactive` logged;
    - a track with stems -> `open_editor` called, `active` after the snapshot;
    - switching tracks calls `close_editor`/`open_editor` and ignores the late snapshot of the first.
  - **Solo:** `lanePlay('guitar')` calls `editor_lane_play(name='guitar')`, and after the returned state the guitar
    lane shows `unmuted` true and `transport.solo` 'guitar'.
  - **Enables:**
    - `canPlay` is false when all stems are unchecked and stopped, true with one checked;
    - `stopEnabled` follows playing/position.
  - **Coalescing:** 20 rapid `setGain` calls with a slow mock result in at most 2 in flight, with the last value sent
    last.
  - **Leaving the view:** switching the view away from the Editor while playing calls `editor_stop` and logs
    `editor leave stop`.
  - **Save:** a `saved` event with `clipped_samples > 0` sets the clip warning message and triggers a library reload.

### Task 16: Editor pane components (lanes, Mix lane, solo, clip, save)
- routine: no
- files: `src/ui/components/editor/{EditorPane,StemLane,MixLane}.svelte` (new),
  `src/ui/components/editor/EditorPane.test.ts` (new), `src/ui/views/EditorView.svelte`
- does:
  - **Lanes**, left to right: label, Play (reads "Solo", amber, `aria-pressed` while soloed), the Unmute checkbox,
    and the dB slider (-60..12, step 0.5, tick at 0) with the value text (a button that resets to 0 dB). Lanes are
    dimmed while another lane is soloed.
  - **Mix lane:** "Mix", Play/Pause, Stop, the position slider (0..duration, step 100 ms, frozen while dragging), the
    `TimeField` slot, "Solo: <Name>" while soloed, the CLIP badge while `clipping`, and Save. Save comes with the
    "Save as: Backing (backings/backing.flac)" text, progress and a Cancel button while saving, and a status/alert line
    (including the clip warning).
  - **States and layout:** the inactive/loading/error states of 2.7; the lanes scroll and the Mix lane is pinned.
  - **Keys:** Space and Ctrl+S in the view.
- done when: the component vitest with `mockIPC` passes for the UI-level acceptance criteria (section 7: AC1-4, 6-8,
  10, the UI part of 16, 17):
  - clicking lane Play checks its Unmute, the lane button reads "Solo", and the Mix button reads "Pause" after the
    transport event;
  - Mix Play is disabled with every lane unchecked (the initial state of a never-saved track) and enabled after
    checking one;
  - Stop is enabled after play;
  - a `transport` event updates the slider value and the time text (`0:05.0`);
  - `clipping: true` shows the CLIP badge;
  - the dB value text shows `Off` at the bottom and `+12.0 dB` at the top;
  - Save is disabled with every lane unchecked, enabled with one, and shows progress on `saving` events;
  - Space toggles play only when the focus is not in a text field/checkbox/slider;
  - `npm test` passes.

### Task 17: Time field (edit and wheel)
- routine: no
- files: `src/ui/components/editor/TimeField.svelte` (new), `src/ui/components/editor/TimeField.test.ts` (new),
  `src/ui/components/editor/MixLane.svelte`
- does: the field of 2.7:
  - it shows `formatPosition`;
  - edit + Enter/blur commits -> `seek`;
  - an invalid value sets `aria-invalid` and a message, with no IPC; Escape reverts;
  - wheel up -> `nudge(-100)`, wheel down -> `nudge(+100)`, with `preventDefault`;
  - events don't overwrite it while it's being edited;
  - `aria-label` "Position (minutes:seconds.tenths)".
- done when: the vitest tests pass:
  - typing `0:05.0` + Enter calls `editor_seek(position=5000)`, and so does `0:05:0`;
  - `9:99.9` and a time past the duration call nothing and set `aria-invalid`;
  - a `wheel` event with `deltaY=-100` calls `editor_nudge(delta=-100)`, and `deltaY=+53` calls
    `editor_nudge(delta=100)`;
  - the wheel event is default-prevented;
  - a transport event doesn't change the field while it holds unsaved text;
  - `npm test` passes.

### Task 18: GUI end-to-end tests
- routine: no
- files: `tests/gui_editor_e2e.rs` (new), `tests/common/mod.rs` (a `start_editor` helper that copies `library-editor`
  like `lib_dirs` and sets `CALLIOPE_E2E_AUDIO=capture:<target/... wav>`), `package.json` (`test:gui` adds
  `--test gui_editor_e2e`)
- does: tests on display `:1`, with the window floated at 1280x800. Keyboard first; the mouse only for the
  slider/wheel, at fixed coordinates in the floated window, verified by the log lines. The scenario:
  1. Open the Editor (Alt+3).
  2. Select "Plain Backing" -> `editor inactive`.
  3. Select "Four Lanes" -> `editor active ... stems=4` and Rust `editor open ... duration_ms=12000`; Mix Play is
     disabled (Space does nothing, no `editor play` line).
  4. "Six Lanes" -> stems=6.
  5. "Missing Stem" -> inactive, with the message (screenshot).
  6. Back on "Four Lanes", check vocals and drums, then Play -> `audio backend=capture`, and the position lines
     advance for 1.5 s.
  7. Pause -> the position is stable for 1 s.
  8. Lane Play on guitar -> `editor solo name=guitar`, and guitar's checkbox is checked (log
     `editor stem name=guitar ... unmuted=true`).
  9. Mix Play -> `solo=none`.
  10. Wheel up 3x over the time field -> 3 `editor nudge delta=-100` lines.
  11. A nudge while playing -> Rust `audible=false`, then `audible=true` 300-450 ms later.
  12. Alt+1 while playing -> `editor leave stop` and Rust `playing=false`.
  13. Back in the Editor, uncheck guitar and Save -> `editor saved` and, on disk, `backings/backing.flac` plus a
      `track.json` `backings[0]` with the checked stems.
  14. Analyse the capture WAV and `backing.flac` (decode with `ffmpeg -f f32le` to stdout, or with claxon). In the
      saved file, the energy at 660 Hz (guitar, unchecked) is below -40 dB relative to 440 Hz (vocals). In the capture
      during the solo, the energy at 440 Hz is below -40 dB relative to 660 Hz.
  15. A second Save with vocals at +12 dB and drums at +12 dB -> the CLIP badge (screenshot) and a `clipped=` count
      > 0; the old file is in `trash/*-backing/`.

  Every test asserts that there is no `csp-violation` and that `calliope: audio backend=` is never `cpal`.
  Screenshots go to `target/gui-shots/`.
- done when: `DISPLAY=:1 npm run test:gui` passes three times in a row (all GUI suites, not only the new one).

### Task 19: Visual review, docs
- routine: no
- files: `docs/ui.md` ("Decided by the team" entries for the Editor), `src/calliope-gui/README.md` (Editor usage,
  `alsa-lib`, the audio fakes), `docs/architecture.md` (only if the implementation deviated from this plan)
- does:
  - Take screenshots of the Editor at 1024x640 and 1920x1200, in dark and light, covering: 4 and 6 lanes, the inactive
    state, playing, solo, CLIP, and saving.
  - Check readability at a distance, control sizes, focus rings, the 0 dB tick, that the solo state is
    distinguishable, and that nothing is cut off at 1024x640 (the Mix lane and Save are visible without scrolling; the
    lanes scroll).
  - Fix the issues found and record the UI decisions.
- done when:
  - the reviewed screenshots are in `target/gui-shots/` and listed in the task log, with the findings fixed;
  - `npm test` and `DISPLAY=:1 npm run test:gui` pass.

## 4. Test strategy

- **Whole headless suite**: `cd src/calliope-gui && npm test` (svelte-check, vitest, vite build,
  `cargo test --workspace`, clippy).
- **GUI suite**: `cd src/calliope-gui && DISPLAY=:1 npm run test:gui`.
- **Audio hardware stand-ins.** Agents never use a real sound device, and no test may make a sound on the owner's
  speakers.
  - `ManualBackend` (unit tests): the test pulls samples from the real render callback and compares them sample by
    sample with the expected mix (including solo and clipping).
  - `NullBackend` (GUI tests): consumes audio in real time on a thread. With `capture:` it writes what would have been
    heard to a WAV that the test analyses (tone energy per stem frequency). Builds with `e2e-hooks` can only use it.
  - `CpalBackend` is compiled and clippy-checked but never constructed by any test (a static test limits where it is
    referenced). Real output is a manual check.
- **Pure logic** (unit tests in the Rust modules):
  - decoding and compatibility rules;
  - dB gain, the clip count, quantisation;
  - transport timing with injected instants (the 0.3 s rule exactly, no sleeps);
  - solo and clip-hold logic with injected time;
  - engine behaviour with fixture stems;
  - render output decoded and compared exactly;
  - the repository save transaction on temp copies (variants, no clobber, trash, conflict, untouched sibling folder).
- **Frontend**: vitest + `mockIPC` for the state, the components, time and gain formatting, wheel and keyboard.
  Transport events are simulated by calling the channel callback.
- **End-to-end**: the real binary on `:1`, with temporary XDG dirs and a copied `library-editor` repository under
  `target/`. Tests assert on log lines and on disk, analyse the capture WAV, and save screenshots for visual review.
- **Fixtures**: `tests/fixtures/library-editor/` (generated by `tests/fixtures/editor/make-fixtures.sh`, committed).
  Unit tests create exact-sample FLACs in temp dirs with `flacenc`.

## 5. Deployment

Single machine: everything runs in calliope-gui on the laptop (`npm run build:app`). New build prerequisite:
`alsa-lib`. Nothing changes on archserver.

## 6. Manual checks (owner, real audio)

1. Build with `npm run build:app` and run `target/release/calliope-gui`. Pick a real imported stem track in the
   Editor: 6 lanes appear, all unchecked at 0 dB, Mix Play is disabled, and "Loading stems" finishes within a few
   seconds.
2. Check every stem except Guitar and click Mix Play. The mix without the guitar is heard on the default output, and
   the time field (`0:12.3` style) and slider move smoothly.
3. While playing, check and uncheck stems, and drag the Vocals volume to -12 dB and then +6 dB. The changes are
   audible at once, and the value text matches.
4. Click Play on the Guitar lane. Only the guitar is heard, its checkbox is checked, the lane button reads "Solo", and
   the other lanes keep their checkboxes. Click Mix Play: the mix returns, guitar included (it is checked now).
   Uncheck Guitar.
5. Push two stems to +12 dB: the CLIP badge lights and you hear the distortion. Bring them back: the badge goes out
   after about a second.
6. Wheel over the time field while playing. The sound stops and resumes about 0.3 s after the last wheel step, from
   the new position. Wheel up goes back and wheel down goes forward, 0.1 s per step.
7. Type `1:00.0` + Enter: playback jumps to one minute (after the short pause). Pause, then Play: it resumes where it
   paused. Stop: back to `0:00.0`.
8. Switch to the Library while playing: playback stops. Select another track in the Editor while playing: playback
   stops.
9. Save: progress shows, then "Saved". Play `<track>/backings/backing.flac` in another player: it matches what you
   heard. The Library shows "Backing tracks: Backing (backings/backing.flac)". Reopen the track after restarting the
   app: the mixer shows the saved settings.
10. Save again with a different mix: the new file replaces it, and the old one is in
    `<repository>/trash/...-backing/`.
11. With a 10-15 minute track, check that loading time and memory use (e.g. in a system monitor) are acceptable.
12. Play to another output set as the system default (e.g. a headset), to confirm the default-device choice.

## 7. Acceptance mapping

| # | Acceptance criterion | Tasks | Tests |
|---|---|---|---|
| 1 | Track without stems -> pane stays inactive | 13, 15, 16, 18 | vitest editor-state/EditorPane (inactive, no `open_editor`); e2e "Plain Backing" + "Missing Stem" |
| 2 | Track with stems -> pane active | 7, 13, 15, 16, 18 | Rust open tests; vitest; e2e 4 and 6 lanes |
| 3 | Lane Play -> its Unmute checked and Mix Play triggered | 7, 15, 16, 18 | Rust `lane_play` checks the stem, plays solo, other checkboxes unchanged; vitest checkbox checked, Mix button "Pause"; e2e solo lines + capture tones |
| 4 | One Unmute checked -> Mix Play enabled | 7, 15, 16, 18 | vitest `canPlay` (disabled when all unchecked = initial state); Rust play refused with none checked; e2e |
| 5 | Play with a checked stem -> the combined stems play | 4, 6, 7, 18 | Rust `ManualBackend` exact mix of checked stems; e2e capture WAV tones |
| 6 | Play started/resumed -> slider and time box update | 7, 11, 16, 17, 18 | Rust event throttle tests; vitest transport event -> slider + `0:05.0`; e2e position lines |
| 7 | Play started -> "Play" becomes "Pause" | 15, 16 | vitest |
| 8 | Play started -> Stop enabled | 15, 16 | vitest; e2e Stop -> position 0 |
| 9 | Pause -> playback pauses at the current position | 5, 7, 18 | Rust: zeros + frozen position after pause, resume continues from there; e2e position stable |
| 10 | Pause -> "Pause" becomes "Play" | 15, 16 | vitest |
| 11 | Edited time within range -> position jumps there | 7, 11, 17 | time-format tests; vitest TimeField -> `editor_seek(5000)`, out of range refused; Rust seek |
| 12 | Wheel up over the time box -> -0.1 s per step | 7, 17, 18 | vitest wheel -> `editor_nudge(-100)`; Rust nudge clamp; e2e 3 steps |
| 13 | Wheel down -> +0.1 s per step | 7, 17 | vitest; Rust |
| 14 | Adjusting while playing -> stop, resume 0.3 s later from the new position | 5, 7, 18 | Rust transport + engine with injected time; e2e `audible` gap 300-450 ms |
| 15 | Adjusting inside the 0.3 s window resets it | 5, 7 | Rust transport (seeks at t0, t0+200 -> resume t0+500) |
| 16 | Save with a checked stem -> backing file from the mix | 8, 9, 10, 16, 18 | Rust render exact-sample test; repository variant/save tests; vitest Save enable/progress; e2e file + `backings[0]` |
| 17 | Volume of a checked lane contributes to the mix | 4, 7, 9, 12, 16, 18 | Rust dB gain + clip tests, engine gain change, render with -6/+6 dB; vitest dB text; e2e +12 dB clip |

Owner decisions covered as well:
- **Solo semantics:** Task 7 and the solo tests in tasks 15, 16 and 18.
- **Stop on view change:** tasks 15 and 18.
- **Variants list:** tasks 8, 13 and 18.
- **`original.flac` hidden:** task 7. "Six Lanes" has no original; a vitest in task 16 renders a record with
  `original` and checks there is no lane for it.

## 8. Open questions

None. The owner answered all of them on 2026-10-08:
- time `3:07.4`;
- volume in dB with boost;
- lane Play = solo;
- start with nothing checked;
- stop on track/view change;
- backing variants list;
- `original.flac` hidden;
- no symphonia;
- FLAC.

Implementation choices made inside those answers, stated here for review:
- **Volume:** -60 dB (= Off) to +12 dB in 0.5 dB steps.
- **Clipping:** hard clip plus a CLIP indicator (1 s hold) and a clipped-sample count after Save, rather than a
  limiter.
- **Solo ends on:** Mix Play, the same lane again, unchecking it, Stop/end, a track change or leaving the view. Pause
  keeps it.
- **Leaving the Editor:** Stop (position back to 0).
- **Default variant:** id `backing`, name "Backing", file `backings/backing.flac`.
- **`audio`:** stays `null` for stem tracks.
- **`M:SS:T` input:** still accepted.
