# Plan: Quicker RC-5 exports

Spec: `specs/gui-export-rc5-quick.md`. Base: `ecea1ce` on branch `spec/gui-export-rc5-quick`. Predecessors:
`specs/gui-export.plan.md` (prepare/start flow, folder checks, overwrite confirmation) and
`specs/gui-export-info.plan.md` ("Last:" lines, Memory box). The spec's "Proposed defaults" (open
questions 1-4) were accepted by the owner when the build started.

## 1. Summary

Once an RC-5 export has succeeded in the current visit of the Export view, the next "to BOSS RC-5"
clicks don't open the folder dialog. They export to the same folder right away, provided it still
exists and still holds `ROLAND/`; if not, the dialog opens as before. The RC-5 row gets a reserved
line "To: <folder>" with a "Change..." button that picks another folder for the next exports.
Rust remembers the folder in memory only. The frontend owns the visit and sends back the folder it
shows. Every existing check, refusal and the overwrite confirmation still run on every export.
Leaving the view, closing the app or reloading the webview ends the visit.

## 2. Design

### 2.1 What is reused, what is new

| Area | Reused | New / changed |
|------|--------|---------------|
| Job | `ExportState` slot, `Planned.base`, `Work::run` success path, `check_destination` | `Inner.rc5_quick: Option<PathBuf>` set on a successful RC-5 job; `ExportState::{rc5_quick, set_rc5_quick}`; `ExportPlan.base`; `check_rc5_base` split out of `check_destination` (2.2) |
| Rules | `export_rules::start_dir` | `quick_rc5_dir(reuse, remembered)` (2.3) |
| IPC | `do_prepare_export` (all checks before the folder, then `ExportState::plan`) | `TargetReq::Rc5.reuse`; the skip branch; `do_choose_rc5_folder` / `choose_rc5_folder` command (2.4) |
| Frontend | `ex` export state, `exportTo`, `answerConfirm`, `applyEvent`, ExportPane rows | `ex.rc5Quick`, `ex.notice`, `beginExportVisit()`, `changeRc5Folder()`; the "To:" row and Change...; `hscroll=` in the layout log (2.5-2.7) |
| Tests | `ScriptedPicker` (answers file + `starts()`), `tests/gui_export_e2e.rs` helpers, `tests/acceptance_export.rs` rig | `ac_quick_*` tests, GUI test `export_rc5_quick`, extended `export_rc5_flow` (2.9) |

### 2.2 Where the folder lives: Rust remembers it, the frontend owns the visit

There are two halves, and both must agree before a dialog is skipped:

- **Rust** (`ExportState`, in memory, never in `settings.json`) remembers the **quick folder**: the
  base folder of the last successful RC-5 export, or the folder last accepted through "Change...".
  - `Work::run` sets it on the success path when `pl.kind == Rc5`, with
    `lock(&self.inner).rc5_quick = Some(pl.base.clone())`, after the rename and before `on_done`.
    A failed or cancelled job never sets it.
  - `pub fn rc5_quick(&self) -> Option<PathBuf>` and `pub fn set_rc5_quick(&self, dir: PathBuf)`
    (the latter for Change...).
  - The process lifetime bounds it, so closing the app always forgets it (requirement 5, out of
    scope "across app restarts").
- **The frontend** (`ex.rc5Quick: string | null` in `export-state.svelte.ts`) holds the visit. It
  is the folder shown on the "To:" line, and it is sent back as `reuse` with each RC-5 export.
  `null` means "ask". It is set only after a success (or a Change...) in this visit, and it is
  cleared when the visit starts (2.6).
- **The skip rule.** Rust skips the dialog only when all of these hold:
  - `reuse` equals its own quick folder (string compare with `to_string_lossy()`);
  - that folder is still a directory;
  - it still has a `ROLAND` directory.

  Otherwise the dialog opens exactly as in requirement 1, at the remembered `rc5_dir`.
- **Why both halves.**
  - Rust alone can't know about views: the view lifecycle is in the frontend.
  - The frontend alone would mean Rust writes into a folder named by the webview. Decision
    2026-10-05 says folders come from Rust-opened dialogs only. With the echo, Rust writes only
    into a folder that a dialog (or a Change... dialog) gave it earlier in this process, and the
    "To:" line always names the folder that will be used. If the two differ for any reason (a
    reload, a job finished after the view was left), the result is a dialog, never a wrong folder.
- `ExportPlan` gains `base: String` (`chosen.to_string_lossy()`): the folder the plan writes into,
  for display. The frontend keeps the base of the RC-5 plan it starts and copies it to
  `ex.rc5Quick` when that job's `done` arrives.
- `check_destination`'s first three checks move into
  `pub fn check_rc5_base(base, repo_root) -> Result<(), (&'static str, String)>` (not-folder,
  no-roland, inside-repo; same messages and reasons). `check_destination` calls it for the RC-5,
  and Change... (2.4) uses it too. Behaviour doesn't change.

### 2.3 Rules: `quick_rc5_dir`

`src/export_rules.rs`:

```rust
/// The folder an RC-5 export may use without the dialog, or why not:
/// "none" (no reuse asked or nothing remembered), "other" (reuse != remembered),
/// "gone" (not a directory any more), "no-roland" (no ROLAND directory in it).
pub fn quick_rc5_dir(reuse: Option<&str>, remembered: Option<&Path>) -> Result<PathBuf, &'static str>;
```

The function is pure apart from two `is_dir` calls. It is unit-tested with temp folders.

### 2.4 IPC (requirements 1-4, 6, 7)

- `TargetReq::Rc5 { memory: String, #[serde(default)] reuse: Option<String> }`. MP3/OGG have no
  such field, so requirement 7 holds by construction. Existing JSON without `reuse` still
  deserializes. Rust call sites write `reuse: None` (three test helpers) or `{ .. }`.
- **`do_prepare_export`**: everything before the folder is unchanged and in the same order (busy,
  memory, tools, track, backing, tempo). Then, for the RC-5 only:
  1. If `reuse` is `Some`: `export_rules::quick_rc5_dir(reuse, export.rc5_quick().as_deref())`.
     - `Ok(dir)`: log `calliope: dialog skipped kind=export-rc5 dir=<dir>`, and `chosen = dir`.
       There is no picker call and the settings are not rewritten (`rc5_dir` already holds it).
     - `Err(reason)`: log `calliope: export quick unavailable reason=<reason> dir=<reuse>`, then
       the normal dialog path (requirement 3), with its `dialog start ... from=last|home|none` line.
  2. If `reuse` is `None`: the normal dialog path (requirement 1).
  3. In every case `export.plan(...)` runs afterwards with all its refusals (busy, tools, probe,
     not-folder, no-roland, inside-repo, is-folder). `plan.exists` drives the unchanged overwrite
     confirmation (requirement 4).
- **`do_choose_rc5_folder(repo, settings, export, home) -> Result<Option<String>, String>`**
  (Change..., requirement 6):
  1. If an export is running: `refused(Rc5, "busy", BUSY_MESSAGE)`.
  2. Open the `ExportRc5` folder dialog (same title). It starts in the quick folder when that is a
     directory (`from=quick`), else `export_rules::start_dir(rc5_dir, home)`. Log the usual
     `dialog start` line.
  3. Cancel gives `Ok(None)`, and nothing changes.
  4. Picked: `check_rc5_base(chosen, current_root)`. A failure is
     `refused(Rc5, reason, message)`, the same message as an export would give ("No ROLAND folder
     in X: is the RC-5 mounted?").
  5. Accepted: `export.set_rc5_quick(chosen)`, `settings.set_rc5_dir(Some(chosen))` (log
     `export remembered kind=rc5 dir=<p>`, as prepare does), log `calliope: export quick dir=<p>`,
     and return `Ok(Some(chosen.to_string_lossy()))`.

  It never exports and never writes on the drive.
- **The command** `choose_rc5_folder() -> Option<String>`, a `#[tauri::command] async` wrapper
  over `do_choose_rc5_folder` through `blocking` (same pattern as `prepare_export`). It is
  registered in `build.rs` `COMMANDS`, `gui.rs` `generate_handler!` and `capabilities/main.json`
  (`allow-choose-rc5-folder`). `tests/frontend.rs` checks that these three agree.

### 2.5 Frontend state (`src/ui/lib/export-state.svelte.ts`, `src/ui/lib/ipc.ts`)

- **Types:**
  - `ExportTargetReq` rc5 becomes `{ kind: 'rc5'; memory: string; reuse: string | null }`;
  - `ExportPlan` gains `base: string`;
  - new wrapper `chooseRc5Folder = (): Promise<string | null> => invoke('choose_rc5_folder')`.
- **New state:**
  - `ex.rc5Quick: string | null` (the "To:" folder);
  - `ex.notice: string` (a Change... refusal, shown on the result line, 2.7);
  - a module variable `startedBase: string | null` (the base of the RC-5 plan being started).
- `setQuick(v)` sets `ex.rc5Quick` and logs `export quick dir=<v|none>` whenever the value
  changes. GUI and vitest tests read this line.
- **`exportTo('rc5', ...)`:**
  - sends `{ kind: 'rc5', memory, reuse: ex.rc5Quick }`;
  - clears `ex.notice`;
  - prepare returns `null`: that only happens when a dialog opened and was cancelled, so
    `setQuick(null)` (spec: "cancelled asks again");
  - a prepare refusal (`Err`) leaves `ex.rc5Quick` as it is. The next click checks the folder
    again anyway, and "Change..." is the way out (2.8);
  - MP3/OGG never send `reuse` and never touch `ex.rc5Quick`.
- **`start(plan, kind)`**: `kind` is passed from `exportTo` and `answerConfirm`, whose `confirm`
  already holds `target`. For the RC-5, `startedBase = plan.base`, otherwise `null`. If
  `startExport` throws for an RC-5, `setQuick(null)` and `startedBase = null`.
- **`applyEvent`**, when the snapshot's `target === 'rc5'`:
  - `done` with a `startedBase`: `setQuick(startedBase)`;
  - `failed` / `cancelled`: `setQuick(null)`.

  In each case `startedBase = null`. Events buffered in `pending` go through the same code.
- **Declined overwrite** (`answerConfirm(false)`): nothing changes. The folder was right, and the
  user is probably fixing the memory number (2.8).
- **`beginExportVisit()`**: `setQuick(null)`, `startedBase = null`, `ex.notice = ''`.
  `ExportView.svelte`'s `onMount` calls it before `attachExport()`. Views are unmounted when another
  one is shown (`App.svelte` `{#if}`), so every arrival in the Export view starts a new visit.
  Because the reset is in `onMount` (not `onDestroy`), a job that finishes while the user is in
  another view cannot carry the skip into the next visit. A webview reload or a new app start
  begins with fresh module state. `resetExport()` also clears the three fields.
- **`changeRc5Folder()`**:
  1. Return at once when an export is running or `ex.preparing`.
  2. Set `ex.preparing = true` (the buttons are disabled while the dialog is open) and clear
     `ex.notice` / `ex.message`.
  3. Call `chooseRc5Folder()`. A string gives `setQuick(r)`. `null` keeps the current folder. An
     error gives `ex.notice = 'Folder not changed: ' + e` and keeps the current folder.
  4. Log `export change result=<picked|cancelled|refused>`.
  5. Finally `ex.preparing = false`.

### 2.6 When the skip starts and ends (spec defaults 1, 4)

| Event in the Export view | `ex.rc5Quick` afterwards |
|---|---|
| View shown (any arrival), app start, webview reload | `null` |
| RC-5 job `done` | the plan's `base` |
| "Change..." picked an accepted folder | that folder |
| "Change..." cancelled or refused | unchanged |
| RC-5 dialog cancelled (`prepare` returned `null`) | `null` |
| RC-5 job `failed` / `cancelled`, or `start_export` error | `null` |
| RC-5 prepare refused (bad memory, no tempo, `ROLAND` gone after the dialog, ...) | unchanged |
| Overwrite declined (No) | unchanged |
| Another track, band or album selected; Memory box edited | unchanged |
| MP3 / OGG export (any outcome) | unchanged |

When `reuse` is sent and Rust falls back to the dialog (folder gone, no `ROLAND`), the outcome of
that dialog export decides, following the table: success gives the new folder, cancel gives `null`.

### 2.7 Layout (requirements 6, 8)

The RC-5 block in `ExportPane.svelte` gets one **reserved row** between the button row and its
"Last:" line:

```
[ to BOSS RC-5 ]  Memory [ 7 ]
To: /run/media/me/BOSS_RC-5            [Change...]   <- new row, min-h-8, always present
Last: memory 7, 2026-10-28
```

- **Markup:**
  - the row is `<div class="flex min-h-8 min-w-0 items-center gap-2" data-testid="export-rc5-to">`;
  - when `ex.rc5Quick` is set, it holds
    `<p class="min-w-0 break-all text-sm" data-testid="export-rc5-folder">To: {ex.rc5Quick}</p>`
    and then
    `<Button variant="outline" size="sm" class="shrink-0" data-testid="export-rc5-change" disabled={running || ex.preparing}>Change...</Button>`;
  - without a folder the row is empty but keeps its height.
- **Position and keyboard:** the row has a fixed height, so the MP3 and OGG buttons always move
  down by the same amount: 2 rem + the 0.25 rem gap = 36 px at 1280x800. They don't jump after the
  first success. Change... comes right after the Memory box in tab order, so the GUI test reaches
  it with a click on the Memory box and then `Tab`.
- **Long paths and narrow widths:** a long path breaks anywhere (`break-all`) and wraps inside the
  pane, and the button stays at the end of the text (`shrink-0`). At 1024x640 the Memory box
  already wraps below its button, and the new row stays one row as long as the path fits.
- **Notice:** the result line shows `ex.message` ("Export failed: ...") first, then `ex.notice`
  (accent colour, `role="alert"`), then `resultText(snap)`.
- **Layout log:** the existing `export layout overflow=<px> width=<px>` line gains
  `hscroll=<scrollWidth - clientWidth>` (measured on the pane). The name doesn't contain
  `overflow=`, so the existing `field(l, "overflow")` parser is unaffected.
- **"Fits" (requirement 8):** at 1280x800, `overflow <= 14` as today and `hscroll <= 0`. At
  1024x640, `hscroll <= 0`, and the pane may scroll vertically, as it already did before this
  feature (docs/ui.md, gui-export-info). Screenshots of both are reviewed.
- **Estimate:** the pane at 1280x800 is about 660 px tall today with about 100 px to spare, so
  the 36 px row should keep `overflow <= 14`. Task 8 measures it. If it doesn't fit, the
  implementer reports it instead of shrinking other rows.

### 2.8 Decisions on the edges (team decisions the owner may overrule)

- **Change... only picks a folder; it doesn't export.** The spec's acceptance line ("Change...
  opens the folder dialog; the folder picked there is used") is met by the next "to BOSS RC-5"
  click using it without a dialog. This matches the usual meaning of a "Change..." button next to
  a destination.
- **Refusals and a declined overwrite keep the skip.** "A failed or cancelled export asks again"
  is read as: the job failed or was cancelled, the dialog was cancelled, or the start failed. A
  typo in the Memory box (refused before any folder) or "No" to an overwrite does not bring the
  dialog back. A destination problem is caught again by the re-check on the next click.
- **The "To:" row is empty until the first success.** Nothing is shown before the first
  success, because the first export always asks.
- **The skip doesn't rewrite `settings.json`** (`rc5_dir` already holds that folder). Change...
  remembers its folder in `rc5_dir` like a dialog export does.

### 2.9 Logs

- **Rust (prefix `calliope: `):**
  - `dialog skipped kind=export-rc5 dir=<p>`;
  - `export quick unavailable reason=<none|other|gone|no-roland> dir=<p|none>`;
  - `export quick dir=<p>` (Change... accepted);
  - the existing `dialog start kind=export-rc5 from=<quick|last|home|none> dir=<p>` (`quick` is
    new, Change... only).
- **Frontend (prefix `calliope-ui: `):**
  - `export quick dir=<p|none>`;
  - `export change result=<picked|cancelled|refused>`;
  - `export layout overflow=<px> width=<px> hscroll=<px>`.

## 3. Tasks

The coordinator runs the full `npm test` (in `src/calliope-gui`) once per task. Implementers run
only the tests named in the task. All Rust is formatted with `cargo fmt --all`.

### Task 1: Export job: the quick folder, `ExportPlan.base`, `check_rc5_base`
- files: `src/calliope-gui/src/export_job.rs`, `src/calliope-gui/src/export_job_tests.rs`
- does:
  - `Inner.rc5_quick: Option<PathBuf>`, set in `Work::run` on success for RC-5 jobs only (2.2);
  - `ExportState::rc5_quick()` / `set_rc5_quick(dir)`;
  - `ExportPlan.base` filled in `plan()` with `p.chosen.to_string_lossy()`;
  - split `pub fn check_rc5_base(base, repo_root)` out of `check_destination` (same order,
    reasons and messages);
  - tests in `export_job_tests.rs`, using the existing rig:
    - a successful RC-5 job sets `rc5_quick` to its base;
    - an MP3 job, a cancelled RC-5 job and a failed RC-5 job (e.g. the drive's `ROLAND` removed
      between plan and start) leave it unchanged;
    - `plan.base` equals the chosen folder;
    - `check_rc5_base` returns `not-folder` / `no-roland` / `inside-repo` and `Ok`.
- done when: `cargo test -p calliope-gui --lib export_job` passes;
  `cargo clippy -p calliope-gui --all-targets -- -D warnings` is clean.
- test files touched: `src/export_job_tests.rs`.
- routine: no

### Task 2: Rule `quick_rc5_dir`
- files: `src/calliope-gui/src/export_rules.rs`
- does: add `quick_rc5_dir` exactly as in 2.3, with unit tests in the module's `tests` on a
  `tempfile` dir:
  - `reuse` `None` → `Err("none")`;
  - nothing remembered → `Err("none")`;
  - different strings → `Err("other")`;
  - a removed folder → `Err("gone")`;
  - a folder without `ROLAND` (or with a `ROLAND` file instead of a directory) → `Err("no-roland")`;
  - a valid folder → `Ok(folder)`.
- done when: `cargo test -p calliope-gui --lib export_rules` passes; clippy is clean.
- test files touched: `src/export_rules.rs` (its `tests` module).
- routine: no (export_rules.rs is a long file)

### Task 3: Export IPC: `reuse`, the skip branch, `choose_rc5_folder`
- files: `src/calliope-gui/src/export_job.rs` (only `TargetReq`),
  `src/calliope-gui/src/export_ipc.rs`, `src/calliope-gui/src/export_ipc_tests.rs`,
  `src/calliope-gui/tests/acceptance_export.rs` (only the `rc5()` helper: `reuse: None`)
- does:
  - `TargetReq::Rc5.reuse` with `#[serde(default)]`;
  - the skip / fallback branch in `do_prepare_export`, with its log lines (2.4);
  - `do_choose_rc5_folder` and the `#[tauri::command] choose_rc5_folder` wrapper (not registered
    yet, task 4);
  - tests in `export_ipc_tests.rs`, all with `ScriptedPicker` and temp dirs:
    - after a successful RC-5 export to `B`, a prepare with `reuse = Some(B)` makes no picker
      request (`starts()` length unchanged, the answers file untouched) and plans
      `B/ROLAND/WAVE/<n>_1/<n>_1.WAV`;
    - a pre-existing target gives `exists = true`;
    - `reuse` set but Rust has no quick folder, a different string, `B/ROLAND` removed, or `B`
      removed: a picker request at the remembered `rc5_dir` (`from=last`);
    - a memory `abc` with a valid `reuse` is still refused before anything;
    - `deserialize` of `{"kind":"rc5","memory":"7"}` gives `reuse: None`;
    - `do_choose_rc5_folder`:
      - cancel gives `Ok(None)` and no change;
      - a folder without `ROLAND` gives `Err` with the no-roland message, and the quick folder and
        `rc5_dir` are unchanged;
      - a valid folder `C` sets the quick folder and `rc5_dir`, and returns `C`;
      - the dialog's start is the quick folder when one is set;
      - while an export runs it is refused as busy.
- done when: `cargo test -p calliope-gui --lib export_ipc` and
  `cargo test -p calliope-gui --test acceptance_export` pass; clippy is clean.
- test files touched: `src/export_ipc_tests.rs`, `tests/acceptance_export.rs` (helper only).
- routine: no

### Task 4: Register the `choose_rc5_folder` command
- files: `src/calliope-gui/build.rs`, `src/calliope-gui/src/gui.rs`,
  `src/calliope-gui/capabilities/main.json`
- does: three one-line insertions, nothing else:
  - `build.rs`: in `const COMMANDS`, after the line `    "prepare_export",` add the line
    `    "choose_rc5_folder",`;
  - `src/gui.rs`: in `generate_handler![...]`, after the line `            export_ipc::prepare_export,`
    add the line `            export_ipc::choose_rc5_folder,`;
  - `capabilities/main.json`: after the line `    "allow-prepare-export",` add the line
    `    "allow-choose-rc5-folder",`.
- done when: `cargo test -p calliope-gui --test frontend` passes (it checks that the three lists
  agree) and `cargo build -p calliope-gui` succeeds.
- test files touched: none (the existing `tests/frontend.rs` checks it).
- routine: yes

### Task 5: Headless acceptance tests
- files: `src/calliope-gui/tests/acceptance_export.rs`
- does: new `ac_quick_*` tests on the existing rig (copies of `tests/fixtures/library-export` in
  temp dirs; the RC-5 is a temp folder with `ROLAND/`). Add a rig helper
  `prepare_reuse(n, memory, reuse: Option<&str>)`. The tests:
  - `ac_quick_first_export_asks_at_the_remembered_folder`: `rc5_dir` set in the settings, and
    `reuse: None` → one `ExportRc5` start at `rc5_dir`;
  - `ac_quick_next_export_skips_the_dialog`: export track A to `B`, then track B with
    `reuse = plan.base` → no new picker request, and the file exists under `B/ROLAND/WAVE/`;
  - `ac_quick_overwrite_still_asks_and_no_writes_nothing`: the skip with an existing memory file
    → `exists == true`, and the plan's path names the file. Not starting the plan leaves the
    drive and repository byte-identical (`repo_unchanged()`, plus a snapshot of `B`);
  - `ac_quick_rc5_gone_opens_the_dialog`: remove `B/ROLAND` → a picker request (script `CANCEL`
    → `Ok(None)`). Same with `B` removed;
  - `ac_quick_change_folder_is_used`: `do_choose_rc5_folder` picks `C`, then a prepare with
    `reuse = C` skips into `C`;
  - `ac_quick_failed_or_cancelled_sets_nothing`: an RC-5 job cancelled (pace) and one failed
    (`ROLAND` removed between plan and start) → `rc5_quick()` stays as before;
  - `ac_quick_mp3_always_asks`: two successful MP3 exports → two `ExportAudio` picker requests.
- done when: `cargo test -p calliope-gui --test acceptance_export` passes.
- test files touched: `tests/acceptance_export.rs`.
- routine: no

### Task 6: Frontend state and types
- files: `src/calliope-gui/src/ui/lib/ipc.ts`, `src/calliope-gui/src/ui/lib/export-state.svelte.ts`,
  `src/calliope-gui/src/ui/views/ExportView.svelte`, `src/calliope-gui/src/ui/lib/ipc-export.test.ts`
- does:
  - the types and `chooseRc5Folder` (2.5);
  - `ex.rc5Quick`, `ex.notice`, `startedBase`, `setQuick`, `beginExportVisit`, `changeRc5Folder`;
  - `reuse` in `exportTo`, and `start(plan, kind)`;
  - the `applyEvent` rules of 2.6;
  - `ExportView.svelte` `onMount` calls `beginExportVisit()` before `attachExport()`;
  - `ipc-export.test.ts`: `chooseRc5Folder` invokes `choose_rc5_folder`, and the rc5 target
    serializes `reuse`.
- done when: `npx vitest run src/ui/lib/ipc-export.test.ts` passes and `npm run check` is clean
  (in `src/calliope-gui`).
- test files touched: `src/ui/lib/ipc-export.test.ts`.
- routine: no

### Task 7: Export pane: "To:" row, Change..., notice, `hscroll`
- files: `src/calliope-gui/src/ui/components/export/ExportPane.svelte`,
  `src/calliope-gui/src/ui/views/ExportView.test.ts`
- does:
  - the reserved row, the `export-rc5-folder` text and the `export-rc5-change` button;
  - the notice on the result line, and `hscroll=` in the layout log (2.7);
  - `ExportView.test.ts`: add `base` to the `plan` literal; make the mock record the `target` of
    each `prepare_export` and answer `choose_rc5_folder`. New tests:
    1. first RC-5 click sends `reuse: null`, and the row has no text;
    2. after `done` of an RC-5 job with plan `base = '/media/B'`, the row reads `To: /media/B`
       and the next click sends `reuse: '/media/B'`;
    3. another track selected → still `reuse: '/media/B'`;
    4. `failed`, `cancelled`, and a `null` prepare (dialog cancelled) each make the next click send
       `reuse: null`;
    5. a prepare refusal and a declined overwrite keep it;
    6. unmount and render again (leaving the view) → `reuse: null`, and the row is empty;
    7. Change... calls `choose_rc5_folder` → `'/media/C'` shows `To: /media/C` and is the next
       `reuse`; `null` keeps `/media/B`; an error shows "Folder not changed: ..." and keeps it;
       Change... is disabled while an export runs;
    8. MP3 after an RC-5 success sends no `reuse`, and the row still shows `/media/B`.
- done when: `npx vitest run src/ui/views/ExportView.test.ts` passes; `npm run check` is clean.
- test files touched: `src/ui/views/ExportView.test.ts`.
- routine: no

### Task 8: GUI end-to-end tests, visual review, docs/ui.md
- files: `src/calliope-gui/tests/gui_export_e2e.rs`, `docs/ui.md`
- does:
  - **Coordinates:** update `BTN_MP3` / `BTN_OGG` for the new row (expected +36 px; take them
    from a screenshot), and add `CHANGE` only if needed: the test reaches Change... by clicking
    `MEMORY_BOX` and then `Tab Return`.
  - **`export_rc5_flow`, second app run:** the first two presses still open the dialog: the visit
    has had no success until the "Yes" export, and "No" keeps that state. Then:
    - the third answer line is removed;
    - after the Yes export, `export quick dir=<base>` is logged;
    - selecting EX_PLAIN with memory 7 logs `calliope: dialog skipped kind=export-rc5`, and no new
      `dialog start` line appears;
    - the confirmation names `007_1/007_1.WAV`. Answer **No** first: the file is byte-identical
      and no `export phase=writing` appears. Then press again (skipped again) and answer Yes, and
      the "one track per memory" assertions follow as today;
    - no `e2e dialog script mismatch` line appears.
  - **New test `export_rc5_quick`** (two drives `B`, `C` made with `rc5()`). Answers:
    `export-rc5 B`, `export-rc5 CANCEL`, `export-rc5 B`, `export-rc5 C`, `export-rc5 B`.
    1. EX_MONO, memory 3, RC-5 → `dialog start kind=export-rc5`, done. `export quick dir=B` is
       logged, and shots are taken: `export-rc5-quick-1280`, then at 1024x640
       `export-rc5-quick-1024`, then back to 1280x800. Check `overflow <= 14` and `hscroll <= 0`
       at 1280, `hscroll <= 0` at 1024.
    2. EX_PLAIN, memory 4, RC-5 → `dialog skipped`, done, and `wav(B, 4)` exists.
    3. Remove `B/ROLAND`, then RC-5 → `export quick unavailable reason=no-roland` and a
       `dialog start` (answer CANCEL), then `export quick dir=none`. Recreate `B/ROLAND`.
    4. RC-5 → dialog (answer B), done.
    5. Change... (click the Memory box, `Tab Return`) → `dialog start kind=export-rc5 from=quick`
       (answer C), then `export quick dir=C`. RC-5 → `dialog skipped ... dir=C`, done, and
       `wav(C, 4)` exists.
    6. Alt+5 (Player), wait for `view=player`, Alt+6, then `export quick dir=none`. RC-5 →
       `dialog start` (answer B), done.
    7. At the end, every answer is used and there is no `script mismatch` line. The repository
       is unchanged apart from `exports` (`snapshot_except_exports`), and
       `assert_no_real_audio`, `no_csp_violation` pass.
  - **`export_mp3_and_ogg`** keeps proving requirement 7 (the second MP3 press opens the dialog).
    Only the coordinates change.
  - **Review the screenshots:**
    - the "To:" text and Change... don't overlap the Memory box or the "Last:" line;
    - the long temp path wraps inside the pane;
    - the MP3/OGG buttons sit where the constants say.
  - **`docs/ui.md`:** add a `(gui-export-rc5-quick)` bullet after the gui-export-info one. It
    covers the reserved "To:" row, Change..., when the dialog is skipped, what ends the skip, and
    the measured 1280x800 / 1024x640 values.
- done when: in `src/calliope-gui`, `npm run build` then
  `npm run test:gui -- -- export_` passes for `gui_export_e2e` (the coordinator runs the full
  `npm run test:gui`), and the screenshots in `target/gui-shots/` were reviewed.
- test files touched: `tests/gui_export_e2e.rs`.
- routine: no

### Task 9: README
- files: `src/calliope-gui/README.md`
- does: in the "## Export" section, insert the following bullet right after the bullet that
  starts with `* "to BOSS RC-5": press the button` (and ends with
  `Eject the RC-5 before unplugging it.`). ASCII only, exactly this text:

  ```
  * Several tracks in a row: once an RC-5 export has succeeded, the next "to BOSS RC-5" presses
    in the Export view write to the same drive without asking, as long as it is still mounted
    there (it still holds `ROLAND/`); otherwise the folder dialog opens again. The line
    "To: <folder>" under the button shows that drive, and "Change..." picks another one. The
    overwrite question is still asked. Leaving the Export view or closing the app forgets it, and
    the next RC-5 export asks again.
  ```
- done when: `grep -n 'Several tracks in a row' src/calliope-gui/README.md` finds the line,
  `LC_ALL=C grep -nP '[^\x00-\x7F]' src/calliope-gui/README.md` prints nothing, and
  `git diff --stat` shows only README.md changed.
- test files touched: none.
- routine: yes

## 4. Test strategy

- **Unit (Rust):**
  - the job's quick folder (set only by a successful RC-5 job), `plan.base` and `check_rc5_base`
    (task 1);
  - `quick_rc5_dir` on temp folders (task 2);
  - prepare's skip and fallback and `do_choose_rc5_folder`, with the `ScriptedPicker`. Its answers
    file and `starts()` prove whether a dialog was opened and where (task 3).
- **Acceptance (Rust, headless):** `tests/acceptance_export.rs` `ac_quick_*`. The RC-5 is a temp
  folder with `ROLAND/`; "unplugged" = `ROLAND` or the folder removed. The real local
  `ffmpeg`/`ffprobe` and the `library-export` fixtures are used, as in the existing tests. Every
  test checks the repository copy (byte-identical for no-write paths, identical apart from
  `exports` after successes).
- **Frontend (vitest + jsdom):** `ExportView.test.ts` with `mockIPC`. It covers the visit (an
  unmount/re-render is "leaving the view"), every row of the table in 2.6, the "To:" row and
  Change..., and MP3 never sending `reuse`.
- **GUI e2e:** `tests/gui_export_e2e.rs` (already in `scripts/test-gui.sh`). The real app runs on
  a private display with scripted folder dialogs (`CALLIOPE_E2E_DIALOG_ANSWERS`) and capture
  audio. "No dialog" = a `dialog skipped` line, no new `dialog start` / `dialog kind=` line, and
  no unused or mismatched answer. Leaving the view is a real Alt+5 / Alt+6. Layout is checked from
  the `export layout` line plus the reviewed screenshots at 1280x800 and 1024x640.
- **Never touched:** a real RC-5, real drives, the user's repository or settings, the real system
  dialogs, the calliope-stems server on 0.0.0.0:8765 or any other host. No symphonia.
- **Whole suite, in `src/calliope-gui`:**
  - `npm test`: `cargo fmt --all --check`, svelte-check, vitest, vite build,
    `cargo test --workspace`, clippy `-D warnings`;
  - `npm run test:gui`: all GUI binaries, each on its own private display.

## 5. Deployment

Laptop app only: `npm run build:app` in `src/calliope-gui` and install as before. There is one
new Tauri command (`choose_rc5_folder`, in the app's own ACL). `settings.json` and `track.json`
are unchanged, since the quick folder is never stored. Nothing changes on archserver or in
calliope-stems.

## 6. Manual checks (owner, laptop with the real RC-5)

1. **First export asks:** mount the RC-5 and start Calliope. In the Export view, export track 1
   to memory 1. The folder dialog opens at the RC-5 drive you used last. Pick it. The RC-5 row
   now shows "To: /run/media/<you>/BOSS_RC-5" (or wherever it is mounted).
2. **Batch:** select track 2, set memory 2, press "to BOSS RC-5". No dialog opens and the result
   says "Written to .../ROLAND/WAVE/002_1/002_1.WAV". Repeat for track 3, memory 3.
3. **Overwrite:** select track 1 again with memory 1. The overwrite question names
   `001_1.WAV`. Answer No: the result line stays as it was and nothing is written. Press again
   (still no folder dialog) and answer Yes.
4. **Unplugged:** eject and unplug the RC-5, press "to BOSS RC-5". The folder dialog opens.
   Cancel it: the "To:" line is gone. Plug the RC-5 back in, export again: the dialog asks once,
   and the following exports skip it.
5. **Change...:** press Change... and pick another folder that is not the RC-5 (no `ROLAND`):
   "Folder not changed: No ROLAND folder in ...". Press Change... again and pick the RC-5: the
   "To:" line shows it.
6. **Leaving the view:** go to the Player (Alt+5) and back (Alt+6). The "To:" line is empty, and
   the next RC-5 export opens the dialog.
7. **MP3:** export an MP3 twice. The dialog opens both times.
8. **On the RC-5:** play memories 1-3 and check that each one is the right track.
9. **Look:** at your usual window size and at a small one (about 1024x640), the "To:" line and
   Change... sit under the RC-5 button without overlapping anything, and the MP3/OGG buttons don't
   move after the first export.

## 7. Acceptance mapping

| Criterion / requirement | Tasks | Tests |
|---|---|---|
| Just opened → "to BOSS RC-5" opens the dialog at the remembered folder (req. 1) | 3, 5, 6, 8 | `ac_quick_first_export_asks_at_the_remembered_folder`; `ExportView.test.ts` test 1; GUI `export_rc5_flow` (`from=last` after the restart), `export_rc5_quick` step 1 |
| Succeeded to `B`, another track → no dialog, file under `B/ROLAND/WAVE/` (req. 2) | 1, 2, 3, 5, 6, 8 | `export_ipc_tests` skip; `ac_quick_next_export_skips_the_dialog`; `ExportView.test.ts` tests 2-3; GUI `export_rc5_quick` step 2, `export_rc5_flow` (EX_PLAIN skipped) |
| Memory file exists → confirmation names it; cancelling writes nothing (req. 4) | 3, 5, 8 | `ac_quick_overwrite_still_asks_and_no_writes_nothing`; `ExportView.test.ts` test 5; GUI `export_rc5_flow` (No under the skip, then Yes) |
| `B/ROLAND` removed → the dialog opens (req. 3) | 2, 3, 5, 8 | `quick_rc5_dir` unit tests; `export_ipc_tests` fallback; `ac_quick_rc5_gone_opens_the_dialog`; GUI `export_rc5_quick` step 3 |
| Player and back → the dialog opens (req. 5) | 6, 7, 8 | `ExportView.test.ts` test 6; GUI `export_rc5_quick` step 6; closing the app: in-memory only (task 1) and GUI `export_rc5_flow` restart |
| Row shows `B`; Change... opens the dialog; the picked folder is used (req. 6) | 1, 3, 4, 5, 6, 7, 8 | `export_ipc_tests` choose tests; `ac_quick_change_folder_is_used`; `ExportView.test.ts` tests 2, 7; GUI `export_rc5_quick` steps 1, 5 |
| Failed or cancelled → the next one opens the dialog (default 1) | 1, 5, 6, 7 | `export_job_tests` (quick folder set only on success); `ac_quick_failed_or_cancelled_sets_nothing`; `ExportView.test.ts` test 4; GUI `export_rc5_quick` step 3 (dialog cancel) |
| MP3 succeeded → MP3 again opens the dialog (req. 7) | 3, 5, 7, 8 | `ac_quick_mp3_always_asks`; `ExportView.test.ts` test 8; GUI `export_mp3_and_ogg` (second MP3 press) |
| 1280x800 and 1024x640 → nothing overflows (req. 8) | 7, 8 | GUI `export_rc5_quick` (`overflow <= 14`, `hscroll <= 0`) + screenshots `export-rc5-quick-1280` / `-1024`; `export_view_buttons`, `export_mp3_and_ogg` keep their checks |
| Out of scope respected (no restart persistence, no mount watching, overwrite kept, MP3/OGG unchanged) | 1, 3 | `settings.json` unchanged by a skip (`ac_quick_next_export_skips_the_dialog` compares it); no new watcher code |

## 8. Open questions

None. These points were decided here, and the owner can overrule any of them (2.8):
- "Change..." only picks the folder for the next exports; it doesn't start an export.
- A refused export (e.g. a bad memory number) and "No" to the overwrite question keep the skip.
  A failed or cancelled job, a cancelled dialog or a failed start end it.
- The "To:" row is reserved but empty until the first success of the visit.
- "Nothing overflows" at 1024x640 means no horizontal overflow and nothing overlapping. The pane
  may still scroll vertically there, as it already did before this feature.
