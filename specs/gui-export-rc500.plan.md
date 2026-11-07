# Plan: Export to the BOSS RC-500

Spec: `specs/gui-export-rc500.md`. Base: `30e8754` on branch `spec/gui-export-rc500`. Predecessors:
`specs/gui-export.plan.md` (RC-5 target, prepare/start, part file + rename), `specs/gui-export-info.plan.md`
(`track.json` `exports`, one track per memory), `specs/gui-export-rc5-quick.plan.md` (quick folder, Change...),
`specs/gui-export-slow-count-in.plan.md` (count-in, pane layout, the `export layout` probe). The owner decided every
open question on 2026-11-07 (listed in the spec): track 2 is `ROLAND/WAVE/0NN_2/0NN_2.WAV`; same WAV format and 1..99;
its own remembered folder, last memory and quick "To:" line; the track choice comes from the track's RC-500 record,
else Track 1; one track per memory + track slot; the Export pane may be re-arranged.

## 1. Summary

The Export view gets a "to BOSS RC-500" button with its own Memory box (1..99) and a Track 1 / Track 2 choice. It writes
the same WAV as the RC-5 to `<root>/ROLAND/WAVE/0NN_<track>/0NN_<track>.WAV`. In Rust it is a fourth export kind,
`rc500`, that reuses the RC-5 code path: WAV writer, ROLAND check, folder creation, overwrite confirmation and quick
folder. It has its own settings (`rc500_dir`, `rc500_memory`), its own in-memory quick folder, its own Change...
command, and its own `track.json` record, `exports.rc500 {memory, track, at}`, with "one track per memory and track"
clearing. The "Export..." group is re-laid out as a two-column grid (RC-5 | RC-500, then MP3 | OGG), so the whole pane
fits at 1280x800 without a scrollbar.

## 2. Design

### 2.1 What is reused, what is new

| Area | Reused | New / changed |
|------|--------|---------------|
| Kinds | `ExportKind`, `TargetReq` (serde tag `kind`) | `ExportKind::Rc500` (`"rc500"`); `ExportKind::is_looper()` (RC-5 and RC-500) and `device()` (`"RC-5"` / `"RC-500"`); `TargetReq::Rc500 { memory, track, reuse }` |
| Paths | `rc5_target`, `parse_memory`, `suggest_memory`, `memory_for`, `quick_rc5_dir` (generic already) | `parse_track`, `rc500_folder_name(n, t)`, `rc500_target(base, n, t)`, `rc500_slot_for(recorded, last)` |
| Audio | `output_format` (RC-5: 44.1 kHz stereo), `WavWriter`, `count_in_plan(map, rate, slow)`, `run` | `output_format(Rc500, _) == (44100, 2)`. No other audio change |
| Job | `ExportState::plan/start`, part file + rename, folders created removed on failure, `rc5_quick` | every `kind == Rc5` test becomes `kind.is_looper()`; `PlanInput/Planned/PlannedRef/Finished` gain `track: Option<u8>`; a second quick folder `rc500_quick`; `quick_dir(kind)` / `set_quick_dir(kind, dir)`; `check_looper_base(kind, base, root)` (`check_rc5_base` stays as its RC-5 form) |
| Record | `exports` in `TrackMeta.extra`, JSON-level `set`, `write_raw`, revision without `exports` | `Rc500Export {memory, track, at}`, `TrackExports.rc500`, `ExportEntry::Rc500`, `rc500_slot`, `clear_rc500`; `Repository::clear_rc500_holders(memory, track, except)` |
| Settings | `rc5_dir`, `rc5_memory` and their lenient load | `rc500_dir`, `rc500_memory`, `set_rc500_dir`, `set_rc500_memory` |
| Dialogs | `DialogKind`, scripted picker | `DialogKind::ExportRc500` (`"export-rc500"`) |
| IPC | `export_info`, `prepare_export`, `start_export`, `choose_rc5_folder` | `ExportInfo.{rc500_memory, rc500_track}`; RC-500 branch in `do_prepare_export`; new command `choose_rc500_folder`; `remember: FnOnce(ExportKind, u8)`; `record_finished` writes and clears RC-500 records |
| Frontend | `ex` state, `exportTo`, `loadInfo`, `lastText`, `exportRows`, `resultText`, the quick logic | `ex.rc500Memory`, `ex.rc500Track`, `ex.rc500Quick`, `changeRc500Folder()`; the quick logic keyed by kind; the RC-500 block; the two-column grid; the `export geometry` log line |

The alternative was a separate RC-500 module or command set. It was rejected: the RC-500 differs from the RC-5 only in
the `_<track>` suffix and in what is remembered. A shared looper path keeps the WAV, the checks and the atomic writes
identical by construction, and that is what the spec asks for ("The WAV format is the RC-5's", "the same confirmation",
"the RC-5's message").

### 2.2 Paths and numbers (spec requirements 1-3)

- `parse_track(t: u8) -> Result<u8, String>`: 1 or 2, else `"Choose Track 1 or Track 2"`.
- `rc500_folder_name(7, 2) == "007_2"`. `rc500_target(base, 7, 2) == base/ROLAND/WAVE/007_2/007_2.WAV`, and
  `rc500_target(base, 7, 1) == rc5_target(base, 7)`. `rc5_target` itself stays as it is.
- `rc500_slot_for(recorded: Option<(u8, u8)>, last: Option<u8>) -> (u8, u8)`: the recorded `(memory, track)` when there
  is one, else `(suggest_memory(last), 1)`. So 99 stays 99, and the first time gives `(1, 1)`.
- The job creates `ROLAND/WAVE` and `0NN_<t>` when they are missing, writes the part file inside `0NN_<t>`, and renames
  it there. It never reads or writes the other track's folder. On failure or cancel it removes only the folders it
  created, as for the RC-5. So track 1 and track 2 never touch each other's files (requirement 4).
- **The ROLAND refusal names the device:** `No ROLAND folder in <base>: is the RC-500 mounted?`. The RC-5's text,
  `... is the RC-5 mounted?`, is unchanged. The refusal has the same reason (`no-roland`), the same check and the same
  sentence; only the device name differs. Saying "RC-5" to someone exporting to an RC-500 would be wrong. Tests assert
  these exact strings. This is a team decision the owner may overrule.

### 2.3 Request and plan (IPC)

- `TargetReq` (JSON): `{kind: "rc500", memory: "<text>", track: 1|2, reuse?: string|null}`. The track is a number. A
  number other than 1 or 2 is refused before the dialog with reason `track`. A non-number is a deserialization error.
- `do_prepare_export`, RC-500 branch, in the same order as the RC-5:
  1. busy;
  2. `parse_memory` (reason `memory`), then `parse_track` (reason `track`);
  3. `require(Rc500)` (ffmpeg/ffprobe, no encoder);
  4. backing, then tempo map;
  5. quick: `quick_rc5_dir(reuse, export.quick_dir(Rc500))`. When it succeeds, log
     `dialog skipped kind=export-rc500 dir=<p>`. Otherwise log `export quick unavailable kind=rc500 reason=<r> dir=<p>`
     and open the dialog: `DialogKind::ExportRc500`, title "Choose the RC-500 drive (its root folder)", starting from
     `start_dir(settings.rc500_dir, home)`;
  6. `export.plan(... memory, track ...)`;
  7. after a plan from the dialog, `settings.set_rc500_dir`, logging `export remembered kind=rc500 dir=<p>`.

  The RC-5 branch is byte-for-byte the same as today, log lines included.
- `choose_rc500_folder() -> Option<string>`: a new Tauri command for Change... on the RC-500 row. Its body is shared with
  `choose_rc5_folder` through a private `do_choose_folder(kind, ...)`:
  - the dialog `export-rc500`, starting from the RC-500 quick folder (`from=quick`), else `rc500_dir`;
  - `check_looper_base(Rc500, ...)`;
  - `set_quick_dir(Rc500, ...)` and `set_rc500_dir`;
  - the log line `export quick kind=rc500 dir=<p>`.

  It never exports. `do_choose_rc5_folder` keeps its signature and log lines. Rejected alternative: a `kind` parameter
  on `choose_rc5_folder`. That changes an existing command for no gain, and the ACL entry per command stays simple.
- `export_info` gains `rc500_memory: u8` and `rc500_track: u8`: `rc500_slot_for(rec.exports.rc500, settings.rc500_memory)`.
  `memory` is still the RC-5 suggestion. `last` gains `rc500`.
- `do_start_export(..., remember: Box<dyn FnOnce(ExportKind, u8) + Send>)`: `on_done` calls it for both looper kinds.
  `remember_memory(settings, kind, n)` saves `rc5_memory` (log `export remembered memory=<n>`, unchanged) or
  `rc500_memory` (log `export remembered kind=rc500 memory=<n>`). The track is not remembered globally, because the
  suggestion without a record is always Track 1 (owner decision 4).
- `ExportSnapshot.target` / `done` events carry `"rc500"`. Nothing else in the events changes.

### 2.4 The record (spec requirement 6)

- `track.json`: `exports.rc500 = {"memory": 1..99, "track": 1|2, "at": "<RFC 3339 UTC>"}`. It is additive, with no
  schema bump. It is kept verbatim by every save and by older builds, because it sits inside `exports` in `extra`, and
  the revision ignores it (2026-10-24 decisions).
- `export_record::read` takes an entry only when both `memory` (an integer 1..99) and `track` (the integer 1 or 2) are
  valid; anything else means no entry. `rc500_slot(obj) -> Option<(u8, u8)>`. `clear_rc500(obj) -> bool` removes it,
  and removes `exports` when it ends up empty (the same rule as `clear_rc5`, through a shared private helper).
- `TrackExports` serializes as `{rc5, rc500, mp3, ogg}`, with `null` for a missing kind.
- `record_finished`, RC-500:
  1. `record_export(id, Rc500 {memory, track}, at)`, log
     `export recorded id=<id> target=rc500 memory=<n> track=<t> at=<at>`;
  2. `clear_rc500_holders(memory, track, id)`. This removes `exports.rc500` from every other track whose record names
     the same memory and the same track. A record on the other track of that memory, or any `rc5` record, is never
     touched. Logs: `export cleared id=<id> target=rc500 memory=<n> track=<t>`, and on failure
     `export clear failed id=<id> target=rc500 memory=<n> track=<t> message=<m>`. The error text is
     `The RC-500 info of "<title>" (memory <n>, track <t>) wasn't cleared: <m>`.

  In the other direction, an RC-5 export clears only `rc5` records, exactly as today.
- `clear_rc500_holders` shares the folder walk of `clear_rc5_holders` through a private helper that takes the match
  predicate, the clear function and the log text. `clear_rc5_holders` keeps its behaviour and its log lines byte for
  byte.

### 2.5 Settings

`settings.json` gains `"rc500_dir": null | "<absolute path>"` and `"rc500_memory": null | 1..99`, read leniently like
`rc5_dir` / `rc5_memory`: an invalid value is logged and treated as none. A missing RC-500 drive is kept, and the dialog
then starts in home. The serialized order is `..., "rc5_memory", "rc500_dir", "rc500_memory"`. Only the `json_shape`
test pins this order.

### 2.6 Frontend state and texts (spec requirements 5-7)

- `ipc.ts`:
  - `ExportKind = 'rc5' | 'rc500' | 'mp3' | 'ogg'`;
  - `ExportTargetReq` gains `{ kind: 'rc500'; memory: string; track: 1 | 2; reuse: string | null }`;
  - `Rc500Export { memory: number; track: number; at: string }`;
  - `TrackExports.rc500?: Rc500Export | null` (optional, so existing test literals stay valid; Rust always sends it);
  - `ExportInfo.rc500_memory: number; rc500_track: number`;
  - `Settings.rc500_dir?`, `rc500_memory?`;
  - `chooseRc500Folder = () => invoke<string | null>('choose_rc500_folder')`.
- `export-state.svelte.ts`:
  - `ex.rc500Memory: string` (box text), `ex.rc500Track: 1 | 2` (default 1), and `ex.rc500Quick: string | null`;
  - `setQuick(kind, v)` replaces the RC-5-only `setQuick`. The RC-5 log stays `export quick dir=<p|none>`; RC-500 logs
    `export quick kind=rc500 dir=<p|none>`;
  - `startedBase` becomes `{ kind, base } | null`;
  - `applyEvent`'s RC-5 rules (set the quick folder on `done`, clear it on `failed` / `cancelled` / a start error / a
    cancelled dialog) apply per looper kind, to that kind's quick folder only;
  - `beginExportVisit()` clears both quick folders;
  - `resetExport()` resets the new fields.
- `loadInfo` fill rule. When the track changes or a refill was asked for, fill all three: `ex.memory = info.memory`,
  `ex.rc500Memory = info.rc500_memory`, `ex.rc500Track = info.rc500_track`. Otherwise fill each memory box only if it
  is empty, and the track together with the RC-500 box. A refill after any export, RC-500 included, therefore shows the
  slot just used (requirement 7). After the existing `export last` line (unchanged), log one more line:
  `export last-rc500 id=<id> memory=<box> track=<1|2> recorded=<m>/<t>|none`.
- `exportTo('rc500', track)` sends `{kind: 'rc500', memory: ex.rc500Memory.trim(), track: ex.rc500Track, reuse:
  ex.rc500Quick}`.
- `changeRc500Folder()` mirrors `changeRc5Folder()`. It logs `export change kind=rc500 result=<picked|cancelled|refused>`
  (the RC-5 line is unchanged) and shows refusals as the same `Folder not changed: ...` notice.
- `export-format.ts`:
  - `lastText('rc500', last)` gives `Last: memory 7, track 2, 2026-11-07` (without a parseable time:
    `Last: memory 7, track 2`);
  - `exportRows` order: RC-5, RC-500, MP3, OGG. The RC-500 row is `{id: 'track-export-rc500', label: 'RC-500', value:
    'memory 7, track 2 (2026-11-07 18:30)'}`, so the Library log `track exports ... rows=rc5,rc500,mp3,ogg` follows
    from the ids with no TrackPane change;
  - `resultText` for `rc500`: `Written to <path>. Eject the RC-500 before unplugging it.`;
  - `confirmText` is unchanged: it names the file.

### 2.7 The Export pane layout (spec requirements 1, 10)

Today the pane fits 1280x800 with zero margin. One more device block stacked like the RC-5 one would add about 120 px,
so the "Export..." group becomes a grid. Measurements below use the app's 18 px rem: the section is 668 px wide at
1280x800 (`export layout width=668`), so the fieldset's inner width is about 594 px.

```
Export...                                   (fieldset, @container)
+----------------------------------+----------------------------------+
| [ to BOSS RC-5     ]             | [ to BOSS RC-500   ] (o) Track 1 |
|                                  |                       ( ) Track 2 |
| Memory [ 7 ] [Change...]         | Memory [ 8 ] [Change...]         |
| To: /run/media/vali/BOSS_RC-5    | To: /run/media/vali/BOSS_RC-500  |  <- min-h-10 (2 lines), text only
| Last: memory 7, 2026-10-24       | Last: memory 8, track 2, 2026-.. |  <- min-h-5
+----------------------------------+----------------------------------+
| [ MP3 file         ]             | [ OGG file         ]             |
| Last: /music/Band - Title.mp3,   | Last: ...                        |  <- min-h-10
+----------------------------------+----------------------------------+
(reason line, min-h-5)
```

- **The grid:** `grid grid-cols-1 gap-x-4 gap-y-2 @min-[576px]:grid-cols-2` inside a fieldset with `@container`.
  - The breakpoint is in px on purpose: rem in container queries is ambiguous with the 112.5 % root.
  - At 1280x800 the container is about 594 px wide, so there are two columns of about 288 px.
  - At 1024x640 it is about 346 px wide, so there is one column (RC-5, RC-500, MP3, OGG stacked; vertical scroll is
    allowed there).
  - With the navigation collapsed it is wider, still two columns.
  - Every cell is `min-w-0`, and long text breaks anywhere (`break-all`), so there is never a horizontal scroll.
- **Buttons** are `h-11 w-40` (180 px; before `w-44`) for all four, so that the RC-500 row fits in the narrowest
  two-column cell: 180 + 13.5 gap + about 80 for the stacked radios ≈ 274 ≤ (576 − 18) / 2 = 279.
- **Device block** (`role="group"`, `aria-label` "BOSS RC-5" / "BOSS RC-500", `data-testid="export-rc5"` /
  `"export-rc500"`), `flex flex-col gap-1`:
  1. the button row, `min-h-11`. The RC-500 has a `RadioGroup.Root` beside the button (`aria-label="RC-500 track"`,
     two items stacked, ids `export-rc500-track-1/2`, labels "Track 1" / "Track 2", `value={String(ex.rc500Track)}`,
     disabled while running or preparing);
  2. the Memory row: label, `Input w-16` and, only when that kind has a quick folder, `Change...` (outline, `sm`).
     Change... moves here from the "To:" row, so the "To:" text gets the whole column width and real mount paths
     stay on one line. DOM order is button, (radios,) Memory box, Change..., so Change... still follows the Memory
     box in tab order, as the rc5-quick GUI test expects;
  3. the "To:" row (`data-testid="export-<kind>-to"`, `min-h-10`, text only, `break-all`,
     `data-testid="export-<kind>-folder"`);
  4. the "Last:" line (`min-h-5`, `data-testid="export-last-<kind>"`).
- **Accessible names:** the RC-5 Memory box keeps `<label for="export-memory">Memory</label>`, so existing tests that
  use `getByLabelText('Memory')` keep working. The RC-500 box (`id="export-rc500-memory"`) shows a visible "Memory"
  as an `aria-hidden` span and has `aria-label="RC-500 memory"`. The RC-500 button's name is "to BOSS RC-500"; queries
  by role use exact names, so they never collide with "to BOSS RC-5". Change... buttons: `data-testid`
  `export-rc5-change` (kept) and `export-rc500-change`.
- **File blocks:** button plus the "Last:" line (`min-h-10`), each in its own cell, as today.
- **Height estimate at 1280x800:** the group's content goes from about 364 px to about 310 px (−54 px). With GUI-test
  paths (two-line "To:" rows, three-line MP3/OGG "Last:" lines) about 30 px of margin is left. If the measurement
  shows overflow > 0, use these levers in this order: (a) the grid's `gap-y-2` → `gap-y-1`; (b) the Band / Album /
  Title grid's `gap-y-1` → `gap-y-0`; (c) the MP3/OGG "Last:" reserve `min-h-10` → `min-h-5`. Record which ones were
  used in `docs/ui.md`.
- **The geometry probe (for the GUI tests):** the pane already logs `export layout ...` once its size settles. In the
  same debounced callback, right after it, it logs
  `export geometry rc5=<x>,<y> memory=<x>,<y> rc500=<x>,<y> rc500_memory=<x>,<y> track1=<x>,<y> track2=<x>,<y>
  mp3=<x>,<y> ogg=<x>,<y> slow=<x>,<y>`. These are the centres of the elements (buttons, the two memory inputs, the two
  radio items, the switch) from `getBoundingClientRect()` × `devicePixelRatio`, rounded. They are window coordinates
  because the webview fills the window. The GUI tests take their constants from this line (task 10). A guard test also
  checks that the constants match it (± 4 px), so the next layout change fails with the new numbers in the message,
  not with clicks that miss.

### 2.8 Logs (additions; every existing line is unchanged)

- Rust:
  - `dialog start kind=export-rc500 from=<last|home|none|quick> dir=<p|none>`, `dialog kind=export-rc500 result=...`
    (the picker), `dialog skipped kind=export-rc500 dir=<p>`;
  - `export quick unavailable kind=rc500 reason=<none|other|gone|no-roland> dir=<p|none>`,
    `export quick kind=rc500 dir=<p>`;
  - `export remembered kind=rc500 dir=<p>`, `export remembered kind=rc500 memory=<n>`;
  - `export recorded id=<id> target=rc500 memory=<n> track=<t> at=<at>`;
  - `export cleared id=<id> target=rc500 memory=<n> track=<t>`, `export clear failed id=<id> target=rc500 memory=<n>
    track=<t> message=<m>`;
  - `export refused target=rc500 reason=<...|track>`;
  - `export start ... target=rc500 path=<.../007_2/007_2.WAV> ...` (the existing format).
- Frontend:
  - `export prepare target=rc500 ...`, `export quick kind=rc500 dir=<p|none>`,
    `export change kind=rc500 result=<...>`;
  - `export last-rc500 id=<id> memory=<box> track=<1|2> recorded=<m>/<t>|none`;
  - `export geometry ...` (2.7);
  - `track exports id=<id> rows=rc5,rc500,mp3,ogg` (whichever exist).

## 3. Tasks

The coordinator runs the full `npm test` (in `src/calliope-gui`) once per task. Implementers run only the tests named
in the task. All Rust is formatted with `cargo fmt --all`. Tests never touch a real RC-5 / RC-500, a real drive, the
system dialogs, an audio device or the user's repository: temp dirs and the scripted picker only. No `symphonia`.
No task changes the RC-5 log lines, texts or paths.

### Task 1: Storage: `exports.rc500` and `clear_rc500_holders`
- files: `src/calliope-gui/src/export_record.rs`, `src/calliope-gui/src/repository.rs`,
  `src/calliope-gui/src/repository_tests.rs`
- does:
  - `export_record.rs`:
    - `Rc500Export { memory: u8, track: u8, at: String }` (Serialize);
    - `TrackExports.rc500: Option<Rc500Export>`, declared right after `rc5`;
    - `ExportEntry::Rc500 { memory: u8, track: u8 }`;
    - `read`: `rc500` as in 2.4;
    - `set`: writes `{"memory", "track", "at"}` under `rc500`;
    - `pub fn rc500_slot(obj: &Map<String, Value>) -> Option<(u8, u8)>`;
    - `pub fn clear_rc500(obj) -> bool`. `clear_rc5` and `clear_rc500` share a private `clear_kind(obj, key)`.
  - unit tests in `export_record.rs`:
    - valid read;
    - lenient read: track 0, 3, `"2"`, 1.5 or `null`, a missing track, memory 0 or 100 all give no entry;
    - `set` keeps `rc5`, `mp3` and unknown keys;
    - `clear_rc500` keeps `rc5`, removes an emptied `exports`, and returns false when there is nothing to remove;
    - `serializes_missing_kinds_as_null` now expects `"rc500": null`.
  - `repository.rs`:
    - the `record_export` log arm for `Rc500` (2.4);
    - `pub fn clear_rc500_holders(&self, memory: u8, track: u8, except: &str) -> ClearOutcome`, sharing the folder walk
      with `clear_rc5_holders` through a private helper (2.4). `clear_rc5_holders` keeps its lines and behaviour.
  - `repository_tests.rs`, new tests:
    - `record_export` with `Rc500` writes only `exports.rc500`; `modified` and the revision are unchanged;
    - `clear_rc500_holders(7, 2, B)`:
      - clears A (7/2);
      - keeps C (7/1), D (`rc5` memory 7) and E (8/2);
      - keeps an `rc5` entry that sits next to A's cleared `rc500`;
      - skips `except` itself;
      - skips a problem track;
      - reports a write failure (`chmod 0555` folder, skipped as root);
    - `clear_rc5_holders(7, ..)` leaves `rc500` 7/1 and 7/2 records alone.
- done when: `cargo test -p calliope-gui --lib export_record` and `cargo test -p calliope-gui --lib repository` pass;
  `cargo clippy -p calliope-gui --all-targets -- -D warnings` is clean.
- test files touched: `src/export_record.rs` (tests), `src/repository_tests.rs`.
- routine: no

### Task 2: Settings and the dialog kind
- files: `src/calliope-gui/src/settings.rs`, `src/calliope-gui/src/picker.rs`
- does:
  - `settings.rs`:
    - fields `rc500_dir: Option<PathBuf>` and `rc500_memory: Option<u8>`, after `rc5_memory`, with doc comments;
    - lenient load: add `("rc500_dir", &mut s.rc500_dir)` to the dir loop, and an `rc500_memory` block like
      `rc5_memory`'s;
    - `set_rc500_dir`, `set_rc500_memory`, like the RC-5 setters;
    - tests: `json_shape` gets `,"rc500_dir":null,"rc500_memory":null` at the end; a new
      `rc500_fields_roundtrip_and_lenient` test does what `export_fields_roundtrip_and_lenient` does for the new
      fields, and checks that setting the RC-500 fields leaves `rc5_dir` / `rc5_memory` unchanged and the other way
      round.
  - `picker.rs`: `DialogKind::ExportRc500` with `as_str() == "export-rc500"`, also in `parse`'s list. A test checks
    that an answers file line `export-rc500 /tmp/x` answers an `ExportRc500` request, and that an `export-rc5` line
    does not (script mismatch).
- done when: `cargo test -p calliope-gui --lib settings` and `cargo test -p calliope-gui --lib picker` pass; clippy is
  clean.
- test files touched: `src/settings.rs` (tests), `src/picker.rs` (tests).
- routine: no

### Task 3: Rules and job: the `rc500` kind
- files: `src/calliope-gui/src/export_rules.rs`, `src/calliope-gui/src/export_job.rs`,
  `src/calliope-gui/src/export_job_tests.rs`, `src/calliope-gui/src/export_ipc.rs` (compile-only, see below)
- does:
  - `export_rules.rs`:
    - `ExportKind::Rc500` (serde `"rc500"`), `as_str`, `is_looper()`, `device()`;
    - `parse_track`, `rc500_folder_name`, `rc500_target`, `rc500_slot_for` (2.2);
    - `output_format(Rc500, _) == (44100, 2)`;
    - update the doc comment of `quick_rc5_dir`: it serves both looper kinds;
    - tests: paths (`007_2`, `099_2`; track 1 equals `rc5_target`); `parse_track` for 1 and 2, refusing 0 and 3;
      `rc500_slot_for` for (recorded, last) = (Some((7,2)), Some(98)) → (7,2), (None, Some(98)) → (99,1),
      (None, Some(99)) → (99,1), (None, None) → (1,1); `output_formats` rows include Rc500; serde.
  - `export_job.rs`:
    - `TargetReq::Rc500 { memory: String, track: u8, #[serde(default)] reuse: Option<String> }`, `kind()`;
    - `PlanInput.track`, `Planned.track`, `PlannedRef.track` and `Finished.track` (`Option<u8>`, `None` except for
      RC-500);
    - `check_looper_base(kind, base, repo_root)` with the device-named ROLAND message (2.2). `check_rc5_base(base,
      root)` calls it with `Rc5`;
    - `check_destination`, `start` (folder creation), `render` (WAV output, no encoder fsync branch) and `require` use
      `kind.is_looper()`;
    - `target_of` handles `Rc500`;
    - `Inner.rc500_quick`; `quick_dir(kind)` / `set_quick_dir(kind, dir)`. `rc5_quick()` / `set_rc5_quick()` stay, as
      wrappers. `Work::run` sets the quick folder of the plan's kind after a success.
  - `export_ipc.rs`, compile-only. Pass `track: None` in its `PlanInput`, and add a temporary `(ExportKind::Rc500, ..)
    => None` arm in `record_finished` (task 4 replaces both). No behaviour change.
  - `export_job_tests.rs`. The rig's `plan` helper takes a `track`. New tests:
    - RC-500 7/1 writes `ROLAND/WAVE/007_1/007_1.WAV`, byte-identical to an RC-5 memory 7 export of the same track to
      another temp drive;
    - RC-500 7/2 writes `007_2/007_2.WAV`. A pre-existing `007_1/007_1.WAV` and `ROLAND/DATA/x` stay byte-identical,
      and only `007_2` was created;
    - a failure (fake failing ffmpeg) or a cancel leaves no `007_2` folder and no part file. `WAVE` stays when it
      existed before;
    - `check_looper_base(Rc500, ...)` gives the reasons and the exact RC-500 ROLAND message; `check_rc5_base` keeps the
      RC-5 text;
    - after an RC-500 success, `quick_dir(Rc500)` is the base and `quick_dir(Rc5)` is still `None`, and the other way
      round;
    - an existing target needs `overwrite` (refused `exists`, file unchanged).
- done when: `cargo test -p calliope-gui --lib export_rules` and `cargo test -p calliope-gui --lib export_job` pass, and
  `cargo test -p calliope-gui --lib export_ipc` still passes unchanged; clippy is clean.
- test files touched: `src/export_rules.rs` (tests), `src/export_job_tests.rs`.
- routine: no

### Task 4: Export IPC for the RC-500
- files: `src/calliope-gui/src/export_ipc.rs`, `src/calliope-gui/src/export_ipc_tests.rs`,
  `src/calliope-gui/tests/acceptance_export.rs` (only the rig's `start` closure and the import of `remember_memory`)
- does:
  - `ExportInfo.{rc500_memory, rc500_track}` filled in `do_export_info` (2.3);
  - the RC-500 branch of `do_prepare_export` (2.3), passing `memory` and `track` into `PlanInput`;
  - `do_choose_folder(kind, ...)` (private), with `do_choose_rc5_folder` (unchanged signature and lines) and the new
    `do_choose_rc500_folder` plus the `#[tauri::command] choose_rc500_folder` wrapper (registered in task 5);
  - `remember_memory(settings, kind, n)` and `do_start_export`'s `remember: Box<dyn FnOnce(ExportKind, u8) + Send>`;
    the `start_export` command passes `move |k, n| remember_memory(&handle.state::<SettingsStore>(), k, n)`;
  - `record_finished`: the RC-500 entry, `clear_rc500_holders`, and the error text (2.4). This replaces the temporary
    arm from task 3;
  - `export_ipc_tests.rs`: the `start` helper uses the new closure. New tests:
    - `info` for a fresh track gives `rc500_memory == 1` and `rc500_track == 1`. After `remember_memory(Rc500, 98)` it
      gives 99/1. A track with an `rc500 {7, 2}` record gives 7/2. `memory` (RC-5) is unaffected by RC-500 settings;
    - prepare RC-500 with no settings: the request is `(ExportRc500, Some(home))`, and after the plan `rc500_dir` is
      the drive and `rc5_dir` is still `None`. A second prepare starts at `rc500_dir` (`from=last`);
    - track 0 and track 3 are refused (`Choose Track 1 or Track 2`) before any dialog; a bad memory is refused before
      any dialog;
    - a folder without `ROLAND` is refused with the exact RC-500 message, and `rc500_dir` is not saved;
    - quick: after an RC-500 success, a prepare with `reuse = base` makes no new picker request. An RC-5 prepare with
      `reuse = that base` opens the RC-5 dialog (`quick unavailable`, reason `none`);
    - `choose_rc500_folder`: picked (it sets the quick folder and `rc500_dir`), cancelled, refused (`no-roland`), busy;
      it starts from the quick folder when there is one;
    - after `done`: `rc500_memory` is saved and `rc5_memory` is untouched. The record is `{memory, track}` and the
      `records` hold the cleared holders.
  - `tests/acceptance_export.rs`: change only the rig's `Box::new(move |n| remember_memory(...))` to
    `Box::new(move |k, n| remember_memory(&SettingsStore::open(path), k, n))`.
- done when: `cargo test -p calliope-gui --lib export_ipc` and `cargo test -p calliope-gui --test acceptance_export`
  pass; clippy is clean.
- test files touched: `src/export_ipc_tests.rs`, `tests/acceptance_export.rs` (rig only).
- routine: no

### Task 5: Register the `choose_rc500_folder` command
- files: `src/calliope-gui/build.rs`, `src/calliope-gui/src/gui.rs`, `src/calliope-gui/capabilities/main.json`
- does: three one-line insertions, nothing else:
  - `build.rs`: in `const COMMANDS`, after the line `    "choose_rc5_folder",` add the line
    `    "choose_rc500_folder",`;
  - `src/gui.rs`: in `generate_handler![...]`, after the line `            export_ipc::choose_rc5_folder,` add the line
    `            export_ipc::choose_rc500_folder,`;
  - `capabilities/main.json`: after the line `    "allow-choose-rc5-folder",` add the line
    `    "allow-choose-rc500-folder",`.
- done when: `cargo test -p calliope-gui --test frontend` passes (it checks that the three lists agree) and
  `cargo build -p calliope-gui` succeeds.
- test files touched: none.
- routine: yes

### Task 6: Headless acceptance tests
- files: `src/calliope-gui/tests/acceptance_export.rs`
- does: new `ac_rc500_*` tests on the existing rig: fixture copies in temp dirs, drives from `Env::rc5()` (a temp folder
  with `ROLAND/`), the real local ffmpeg/ffprobe. Rig additions:
  - `rc500(m, t) -> TargetReq`;
  - `rc500_wav_path(base, n, t)`;
  - `answer` writes `export-rc500` for `TargetReq::Rc500`.

  Tests:
  - `ac_rc500_memory_7_track_1`: writes `B/ROLAND/WAVE/007_1/007_1.WAV`. The WAV header is checked byte-exact as in
    `ac_rc5_memory_7` (float, stereo, 44.1 kHz, 44-byte header). The file is byte-identical to an RC-5 memory 7 export
    of the same track to drive A. `repo_unchanged_except_exports`;
  - `ac_rc500_track_2_leaves_track_1_alone`: `007_1/007_1.WAV` (pre-written bytes) and `ROLAND/DATA/MEMORY1.RC0`
    (dummy) are present. Track 2 writes `007_2/007_2.WAV`, and both other files are byte-identical. Then track 1 is
    exported over the existing `007_1` (with confirmation), and `007_2.WAV` is byte-identical afterwards;
  - `ac_rc500_overwrite_needs_confirmation`: with `007_2.WAV` present, the plan has `exists == true` and its `path`
    ends with `ROLAND/WAVE/007_2/007_2.WAV`. `start(token, false)` is refused and the file is unchanged ("No" never
    calls start). `start(token, true)` replaces it;
  - `ac_rc500_without_roland`: refused with exactly `No ROLAND folder in <B>: is the RC-500 mounted?`. Nothing is
    written, `rc500_dir` is not saved, and the RC-5 refusal text is unchanged in the same test;
  - `ac_rc500_quick_is_its_own`: after an RC-500 success to B, the next RC-500 prepare (another track, `reuse = B`) asks
    no dialog (`ScriptedPicker::starts()` length unchanged) and writes to B. An RC-5 prepare with no reuse asks
    (`ExportRc5` request);
  - `ac_rc500_and_rc5_folders_survive_a_restart`: pick R for the RC-5 and B for the RC-500. A new `Env` state on the
    same temp settings file (restart): the next RC-5 dialog starts at R and the next RC-500 dialog at B (from
    `starts()`);
  - `ac_rc500_one_track_per_memory_and_track`: A exported to 7/2, then B to 7/2. A has no `rc500` record. C (7/1,
    written with `write_exports`) and D (`rc5` memory 7) keep theirs, and B's `done.records` holds A. An RC-5 export of
    E to memory 7 leaves B's `rc500` record alone;
  - `ac_rc500_info_follows_the_record`: the track exported to 7/2 gives `info.rc500_memory == 7`,
    `info.rc500_track == 2`, `info.last.rc500 == {7, 2, at}`. A never-exported track gives `(last + 1, 1)`, and 99
    stays 99;
  - `ac_rc500_slow_count_in`: with `slow(n, true)`, the RC-500 WAV has the slow layout: zeros up to 176400, tick onsets
    at `176400 + {0, 44100, 88200, 110250, 132300, 154350}` ± 3, and the original from 352800;
  - `ac_rc500_bad_memory_and_track_refused_before_the_dialog`: memory `0` / `100` / `abc` and track 0 / 3 are refused
    with no picker request and nothing written.
- done when: `cargo test -p calliope-gui --test acceptance_export` passes.
- test files touched: `tests/acceptance_export.rs`.
- routine: no

### Task 7: Frontend types, state, texts and the Library row
- files: `src/calliope-gui/src/ui/lib/ipc.ts`, `src/calliope-gui/src/ui/lib/export-state.svelte.ts`,
  `src/calliope-gui/src/ui/lib/export-format.ts`, `src/calliope-gui/src/ui/lib/export-format.test.ts`,
  `src/calliope-gui/src/ui/lib/ipc-export.test.ts`, `src/calliope-gui/src/ui/views/TrackPane.test.ts`,
  `src/calliope-gui/src/ui/views/ExportView.test.ts` (only the `info` literal near line 64: add `rc500_memory: 1,
  rc500_track: 1`)
- does:
  - `ipc.ts`, `export-state.svelte.ts` and `export-format.ts` as in 2.6. The pane is untouched here, so the RC-5 flow
    keeps working;
  - `export-format.test.ts`:
    - `lastText('rc500', ...)` with and without a time;
    - `exportRows` with all four kinds (order and ids) and with `rc500` only;
    - `resultText` for an `rc500` snapshot;
  - `ipc-export.test.ts`: `chooseRc500Folder()` invokes `choose_rc500_folder`; `prepareExport('t1', {kind: 'rc500',
    memory: '7', track: 2, reuse: null})` passes the target through unchanged;
  - `TrackPane.test.ts`:
    - a track with `exports.rc500 = {memory: 7, track: 2, at: '2026-11-07T17:30:00Z'}` shows a row labelled "RC-500"
      with the value `memory 7, track 2 (<local stamp>)`, using `formatStamp` for the expectation;
    - with both `rc5` and `rc500` the rows come in the order RC-5, RC-500;
    - the `track exports ... rows=rc5,rc500` log line.
- done when: in `src/calliope-gui`, `npx vitest run src/ui/lib/export-format.test.ts src/ui/lib/ipc-export.test.ts
  src/ui/views/TrackPane.test.ts src/ui/views/ExportView.test.ts` passes and `npm run check` is clean.
- test files touched: the four test files above.
- routine: no

### Task 8: The Export pane: RC-500 block and the two-column layout
- files: `src/calliope-gui/src/ui/components/export/ExportPane.svelte`, `src/calliope-gui/src/ui/views/ExportView.test.ts`
- does:
  - the layout, the RC-500 block, the RC-5 block changes (Change... on the Memory row, `w-40` buttons) and the
    `export geometry` log line, exactly as in 2.7. The `export last-rc500` line comes from `loadInfo` (task 7);
  - `ExportView.test.ts`. The mock answers `choose_rc500_folder` like `choose_rc5_folder`, and `info()` returns
    `rc500_memory` / `rc500_track`. New tests:
    1. an available track enables "to BOSS RC-500". The RC-500 memory box (`getByLabelText('RC-500 memory')`) shows
       `info.rc500_memory`, and the radio named "Track 1" is checked. `getByLabelText('Memory')` still finds exactly
       the RC-5 box;
    2. info with `rc500_memory: 7, rc500_track: 2` shows 7 and "Track 2" checked. Selecting another track (1/1) shows
       that track's values;
    3. click "Track 2", type `12`, press the RC-500 button. `prepare_export` gets `{kind: 'rc500', memory: '12',
       track: 2, reuse: null}`;
    4. after a `done` of an RC-500 job, the RC-500 "To:" row shows the base and `export-rc500-change` appears, while
       the RC-5 row stays empty and the next RC-5 click sends `reuse: null`. The next RC-500 click sends `reuse: base`.
       A failed RC-500 job clears only the RC-500 folder;
    5. Change... on the RC-500 row: picked / cancelled / refused (notice `Folder not changed: ...`), and disabled while
       running;
    6. the radios and the RC-500 box are disabled while preparing and while an export runs. The RC-500 button is
       disabled with the same reasons as the others (group row, unavailable, running);
    7. a `done` RC-500 result line reads `Written to <path>. Eject the RC-500 before unplugging it.`, and the
       "Last:" line under the RC-500 button shows `Last: memory 7, track 2, <day>` after the refill;
    8. leaving and re-entering the view (unmount + render) clears both quick folders.

    The existing tests must pass unchanged, except where a test depended on Change... being inside the "To:" row: none
    does today (`row()` only checks text).
- done when: `npx vitest run src/ui/views/ExportView.test.ts` passes; `npm run check` is clean; `npm run build` succeeds.
- test files touched: `src/ui/views/ExportView.test.ts`.
- routine: no

### Task 9: README
- files: `src/calliope-gui/README.md`
- does: two edits in the "## Export" section, ASCII only, nothing else:
  1. Replace the first paragraph's first sentence `The Export view (Alt+6) copies the backing track of the track
     selected in the tree to the BOSS RC-5` + `loop station, or saves it as an MP3 or an OGG file.` (it spans two
     lines) with:

     ```
     The Export view (Alt+6) copies the backing track of the track selected in the tree to the BOSS RC-5
     or BOSS RC-500 loop station, or saves it as an MP3 or an OGG file.
     ```
  2. Right after the bullet that ends with `the next RC-5 export asks again.`, insert this bullet:

     ```
     * "to BOSS RC-500": the same as the RC-5, with its own Memory box and a choice of
       "Track 1" or "Track 2". Memory 7, track 2 is written to `ROLAND/WAVE/007_2/007_2.WAV`
       (track 1 to `007_1/007_1.WAV`); the other track's file is never touched. The RC-500
       has its own remembered drive, its own last memory (the box suggests it plus one, with
       Track 1) and its own "To: <folder>" line and "Change...", independent of the RC-5.
       A track exported to the RC-500 shows its memory and track again when selected, and the
       Library shows an "RC-500" row. One track per memory and track: exporting to memory 7,
       track 2 removes the RC-500 entry of any other track on memory 7, track 2 (not track 1,
       and not the RC-5 entries).
     ```
- done when: `grep -c 'to BOSS RC-500' src/calliope-gui/README.md` prints 1, `grep -c 'or BOSS RC-500 loop station'
  src/calliope-gui/README.md` prints 1, `LC_ALL=C grep -nP '[^\x00-\x7F]' src/calliope-gui/README.md` prints nothing,
  the file ends with a newline, and `git diff --stat` shows only README.md changed.
- test files touched: none.
- routine: yes

### Task 10: GUI end-to-end tests, measurement, visual review, docs/ui.md
- files: `src/calliope-gui/tests/gui_export_e2e.rs`, `docs/ui.md`
- does:
  - **Measure first.** Run `npm run test:gui -- -- export_view_buttons` once. Read the newest
    `calliope-ui: export geometry` line in `target/gui-logs/gui_export_e2e.log` at 1280x800, and set from it:
    - `BTN_RC5`, `MEMORY_BOX`, `BTN_MP3`, `BTN_OGG` and `SLOW_SWITCH` (expected unchanged);
    - the new `BTN_RC500`, `RC500_MEMORY_BOX`, `RC500_TRACK1` and `RC500_TRACK2`.

    The comments say where the numbers come from. Add a helper `geometry(app) -> BTreeMap<String, (i32, i32)>` that
    parses the newest line. In `export_view_buttons`, after the 1280x800 shot, add a guard: every constant is within
    ± 4 px of its geometry entry (the message prints the measured values).
  - **Existing tests:** they keep their logic. Only the constants move. Run every `export_*` test. `export_rc5_quick`
    keeps `o1280 == 0`, and its step 5 "Memory box, Tab, Return" still reaches Change....
  - **New helper** `rc500_wav(base, n, t)`.
  - **New test `export_rc500_flow`**:
    - Setup: drives `B = rc5(&dirs, "B")` (RC-500) and `R = rc5(&dirs, "R")` (RC-5). `B/ROLAND/WAVE/007_1/007_1.WAV` is
      pre-written with known bytes. Answers: `export-rc500 <B>`, `export-rc5 <R>`, `export-rc500 <B>`,
      `export-rc5 <R>`.
    1. Select EX_MONO. Set the RC-500 memory to 7 (click `RC500_MEMORY_BOX`, `ctrl+a`, type), click `RC500_TRACK2`,
       press `BTN_RC500`. Expect `dialog start kind=export-rc500`, then done. `rc500_wav(B, 7, 2)` exists
       with the RC-5 header (`read_wav`), and `007_1/007_1.WAV` is byte-identical. The log shows
       `export recorded id=<EX_MONO> target=rc500 memory=7 track=2` and the frontend
       `export quick kind=rc500 dir=<B>`.
    2. Select EX_PLAIN. Memory 8, Track 1, press `BTN_RC500`: `dialog skipped kind=export-rc500 dir=<B>`, no new
       `dialog start kind=export-rc500`, and `rc500_wav(B, 8, 1)` exists.
    3. `set_memory("3")`, press `BTN_RC5`: `dialog start kind=export-rc5` (it asks), done, `wav(R, 3)`.
    4. Layout with both "To:" rows filled, the RC-5 and RC-500 "Last:" lines and a result line: shots
       `export-rc500-1280`, then `layout_at` 1024x640 → `export-rc500-1024`, then back to 1280x800. Write
       `target/gui-shots/export-rc500-layout.txt`. Assert `o1280 == 0`, `h1280 <= 0` and `h1024 <= 0`.
    5. Select EX_MONO again: `export last-rc500 id=<EX_MONO> memory=7 track=2 recorded=7/2`. Shot `export-rc500-recall`
       (Track 2 shown selected). Alt+1 (Library): `track exports id=<EX_MONO> rows=rc500` (plus whatever else EX_MONO
       has), shot `library-rc500-row`. Back with Alt+6.
    6. Overwrite declined: EX_MONO, memory 7, Track 2, `BTN_RC500` (quick, no dialog). Expect
       `export prepare target=rc500 result=plan exists=true`; press `Escape`, or `Return` on the focused No, as in
       `export_rc5_flow`; then `export confirm answer=no`. `007_2.WAV` is byte-identical to after step 1.
    7. Restart (second app run on the same dirs). Press `BTN_RC500` on EX_PLAIN: `dialog start kind=export-rc500
       from=last dir=<B>`. Press `BTN_RC5`: `dialog start kind=export-rc5 from=last dir=<R>`. Let both complete.
    8. Every answer was used, no `script mismatch`, `snapshot_except_exports` is unchanged (`player.json` excluded as in
       `export_rc5_quick`), `assert_no_real_audio`, `no_csp_violation`.
  - **Slow count-in on the RC-500:** in `export_slow_count_in`, no change. The slow layout for the RC-500 is proven
    headless in task 6, and the job path is the same.
  - **Review the screenshots:**
    - at 1280x800, RC-5 and RC-500 sit side by side and MP3 and OGG under them;
    - the radios sit beside the RC-500 button, and nothing overlaps or is cut;
    - the RC-500 "Last:" line is one line;
    - at 1024x640 the cells stack in one column with no horizontal scroll;
    - `library-rc500-row` shows `RC-500  memory 7, track 2 (...)`.
  - **`docs/ui.md`:** add a `(gui-export-rc500)` bullet after the slow-count-in one. It covers the grid and its
    576 px breakpoint, `w-40` buttons, Change... on the Memory row, the RC-500 block (radios, accessible names), the
    texts, the `export geometry` line, the new button centres at 1280x800, which (if any) levers of 2.7 were used, and
    the measured 1280x800 / 1024x640 values. Also update the gui-export-info sentence that lists the Library rows to
    mention the "RC-500" row.
- done when: in `src/calliope-gui`, `npm run test:gui -- -- export_` passes (the coordinator runs the full
  `npm run test:gui`), and the screenshots in `target/gui-shots/` were reviewed.
- test files touched: `tests/gui_export_e2e.rs`.
- routine: no

## 4. Test strategy

- **Unit (Rust):**
  - `export_record` (rc500 read / set / clear and leniency, serialization) and `repository` (`record_export` Rc500,
    `clear_rc500_holders` vs `clear_rc5_holders`): task 1;
  - `settings` (new keys, lenient, independence) and `picker` (`export-rc500`): task 2;
  - `export_rules` (paths, track, suggestion) and `export_job` (target folders, other files untouched, cleanup, quick
    per kind, device-named refusal): task 3;
  - `export_ipc` (info, prepare, choose, remember, record): task 4.
- **Acceptance (Rust, headless):** the `ac_rc500_*` tests in `tests/acceptance_export.rs` (task 6). They run on fixture
  copies in temp dirs with the real local ffmpeg/ffprobe. An RC-500 drive is a temp folder with `ROLAND/`, exactly like
  the RC-5 fake, and "unplugged" is `ROLAND/` removed. "Restart" means new state objects on the same temp
  settings/repository. "No dialog" is proved with `ScriptedPicker::starts()`. Every existing `ac_*` test runs unchanged
  as the RC-5 / MP3 / OGG regression (requirement 9).
- **Frontend (vitest + jsdom):** `export-format.test.ts`, `ipc-export.test.ts`, `TrackPane.test.ts` (task 7), and
  `ExportView.test.ts` (task 8: RC-500 controls, target sent, suggestions, independent quick folders, Change..., disabled
  states, texts).
- **GUI e2e:** `tests/gui_export_e2e.rs` `export_rc500_flow` runs the real app on a private display with scripted
  `export-rc500` / `export-rc5` dialogs and capture audio. It covers real clicks on the radios and the button, the WAV
  on the temp drive, the quick skip, the restart, the overwrite "No", the Library row, and the layout lines plus
  screenshots at 1280x800 and 1024x640. Coordinates come from the `export geometry` line, with a guard in
  `export_view_buttons`. All other `export_*` GUI tests run as regression.
- **Never touched:** a real RC-5 or RC-500, real drives, the user's repository or settings, the real system dialogs,
  any audio device, archserver. No symphonia.
- **Whole suite, in `src/calliope-gui`:**
  - `npm test`: `cargo fmt --all --check`, svelte-check, vitest, vite build, `cargo test --workspace`, clippy
    `-D warnings`;
  - `npm run test:gui`: all GUI binaries, each on its own private display.

## 5. Deployment

Laptop app only: `npm run build:app` in `src/calliope-gui` and install as before. There is one new Tauri command
(`choose_rc500_folder`, in the app's own ACL). `settings.json` gains `rc500_dir` / `rc500_memory`, which older builds
ignore. `track.json` gains the optional `exports.rc500`, which older builds keep verbatim and ignore. Nothing changes
on archserver or in calliope-stems.

## 6. Manual checks (owner, laptop with the real RC-500 and RC-5)

1. **Track 1:** mount the RC-500 and pick a track with a tempo. Set Memory 7 and Track 1, then press "to BOSS RC-500"
   and choose the drive root (the folder with `ROLAND`). "Written to .../ROLAND/WAVE/007_1/007_1.WAV. Eject the
   RC-500 ..." appears. Eject, then on the pedal select memory 7 and play track 1: you hear the silence, the count-in,
   then the track.
2. **Track 2 keeps track 1:** mount again and pick another track. Set Memory 7 and Track 2 and export. The dialog does
   not open if you stayed in the Export view (it says "To: <drive>"). Eject. On memory 7, both tracks play their own
   backing: track 1 is still the first one. If the pedal shows track 2 as empty or refuses it, tell us (the memory's
   own settings file is not written, by design).
3. **Overwrite:** export to memory 7, Track 2 again. The question names `.../007_2/007_2.WAV`. Answer No: nothing
   changes on the pedal. Answer Yes on a second try: the new file plays.
4. **Remembered:** select the first track again. The RC-500 box shows 7 with Track 1 selected, and "Last: memory 7,
   track 1, <date>" is under the button. In the Library, the track has a row "RC-500 memory 7, track 1 (<date time>)".
   For a track never exported to the RC-500, the box shows the last memory used plus one, with Track 1.
5. **Separate drives:** close and restart Calliope. "to BOSS RC-500" opens the dialog at the RC-500 drive, and "to BOSS
   RC-5" at the RC-5 drive (mount both, or one after the other).
6. **RC-5 regression:** export a track to the RC-5, memory 3. The file is `ROLAND/WAVE/003_1/003_1.WAV` as before,
   plays on the RC-5, and the RC-5 "To:" line and Change... work as before. An RC-5 export to memory 7 does not remove
   any RC-500 "memory 7" row in the Library.
7. **Slow count-in:** with "Slow count-in (half tempo)" on, an RC-500 export has the slow count-in on the pedal.
8. **Look:** at your usual window size, the RC-5 and RC-500 blocks sit side by side with MP3 / OGG under them, and
   there is no scrollbar. At a small window (about 1024x640) they stack and only scroll vertically.

## 7. Acceptance mapping

| Criterion / requirement | Tasks | Tests |
|---|---|---|
| Memory 7, Track 1 → `B/ROLAND/WAVE/007_1/007_1.WAV` in the RC-5's WAV format (req. 1, 2) | 3, 4, 6, 8, 10 | `export_job_tests` RC-500 7/1 byte-identical to RC-5; `ac_rc500_memory_7_track_1`; `ExportView.test.ts` 3; GUI `export_rc500_flow` step 2 |
| Memory 7, Track 2 → `007_2/007_2.WAV`; existing `007_1.WAV` byte-identical (req. 2, 4) | 3, 6, 10 | `export_job_tests` 7/2 test; `ac_rc500_track_2_leaves_track_1_alone`; GUI step 1 |
| `007_2.WAV` exists → the confirmation names it; "No" writes nothing (req. 4) | 3, 6, 10 | `export_job_tests` overwrite; `ac_rc500_overwrite_needs_confirmation`; GUI step 6 |
| No `ROLAND` → refused with the RC-5's message (req. 3) | 3, 4, 6 | `check_looper_base` test; `export_ipc_tests` no-roland; `ac_rc500_without_roland` (exact text, device-named; see 2.2) |
| RC-500 success → next RC-500 export in the visit skips the dialog; an RC-5 export still asks (req. 5) | 3, 4, 7, 8, 6, 10 | `export_job_tests` quick per kind; `export_ipc_tests` quick; `ac_rc500_quick_is_its_own`; `ExportView.test.ts` 4, 8; GUI steps 2-3 |
| RC-500 and RC-5 folders picked → after a restart each dialog opens at its own folder (req. 3) | 2, 4, 6, 10 | `settings` independence test; `export_ipc_tests` prepare; `ac_rc500_and_rc5_folders_survive_a_restart`; GUI step 7 |
| A on 7/2, B exported to 7/2 → A loses its RC-500 record; 7/1 and RC-5 memory 7 keep theirs (req. 6) | 1, 4, 6 | `repository_tests` `clear_rc500_holders`; `export_ipc_tests` done records; `ac_rc500_one_track_per_memory_and_track` |
| Track exported to 7/2, selected → Library "RC-500 memory 7, track 2 (...)", Export view memory 7 and Track 2 (req. 6, 7) | 4, 7, 8, 6, 10 | `export_ipc_tests` info; `ac_rc500_info_follows_the_record`; `TrackPane.test.ts`; `ExportView.test.ts` 2, 7; GUI step 5 |
| No record → last RC-500 memory + 1 (99 stays 99) and Track 1 (req. 7) | 3, 4, 6, 8 | `rc500_slot_for` unit test; `export_ipc_tests` info; `ac_rc500_info_follows_the_record`; `ExportView.test.ts` 1 |
| Slow switch on → the RC-500 file has the slow count-in (req. 8) | 3, 6 | `ac_rc500_slow_count_in` (exact frames) |
| RC-5, MP3 and OGG unchanged (req. 9) | all | every existing `ac_*`, `export_*` unit, vitest and GUI test, unchanged except for coordinates; RC-5 log lines unchanged |
| 1280x800: no scrollbar with every line; 1024x640: no horizontal scroll (req. 10) | 8, 10 | GUI `export_rc500_flow` step 4 (`overflow == 0`, `hscroll <= 0`) + screenshots; `export_rc5_quick` and `export_slow_count_in` keep `o1280 == 0` |

## 8. Open questions

None. The following were decided here, and the owner can overrule any of them:
- **The ROLAND refusal names the device** ("is the RC-500 mounted?"). It is the same check and the same sentence as
  the RC-5's.
- **Only the RC-500 memory is remembered globally**, not the track, because the suggestion without a record is always
  Track 1.
- **Any export refills both device boxes** from the track's info, as an MP3 export refills the RC-5 box today.
- **Layout:** RC-5 | RC-500 side by side and MP3 | OGG side by side in a two-column grid that stacks below a 576 px
  wide group. Change... moves from the "To:" row to the Memory row on both devices. The export buttons go from `w-44`
  to `w-40`.
