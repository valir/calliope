# Report: Local import, smarter Browse and whole-directory import

**Status: DONE.** All acceptance criteria pass, the tester found no defects and the reviewer approved with no fix
round. The full headless suite (647 frontend tests and all Rust tests) passes on `9d80d14`. The full GUI suite
passed on `09a2116`, with one exception: an older stem-extraction test stalled once while the app was starting.
It passes when run alone (see Findings). Branch `spec/gui-import-local-batch`, base `ab99cc6`. Nothing is pushed or
merged.

## What was built

- **Where Browse opens** (Local Audio File and Local Video File):
  - a valid path in the box: an existing file opens its folder, and an existing folder opens itself. `~` and
    relative paths are read the same way as for Extract;
  - otherwise the folder of the last local import that started. It is saved as `last_import_dir` in
    `settings.json`, survives a restart and is forgotten silently once that folder is gone;
  - otherwise the home directory;
  - invalid text in the box is never an error for Browse.
  - One remembered folder is shared by audio, video and directory imports. The dialog only opens the folder; it
    doesn't select the file.
  - After a file import it is the file's folder. After a directory import it is that directory itself (your
    answer).
- **Local Directory**, the fourth source option: a path box, Browse (opens the system *folder* dialog) and
  Extract (Enter also starts it).
  - **Which files:** only the audio files directly in the folder, by extension, ignoring upper/lower case.
    Subfolders, hidden files (`.x`, `._x.mp3`) and other files are skipped. They are imported in file-name order,
    ignoring case.
  - **Each file:** goes through the normal import steps (preparation, the file's own tags, stem extraction,
    empty stems dropped, track saved). There is no edit pane; the file name without its extension is the title
    when the file has no tags.
  - **Errors:** "Directory <dir> has no audio files" when there are none. A missing folder or a file path gives
    an error under the box, and nothing starts.
  - **While it runs:** "File 3 of 8: song.mp3" with the current file's progress, a list of the files done, and
    Cancel. The footer reads "File i of n: <phase>".
  - **A failing file** is listed with its reason, and the next file starts.
  - **Cancel** stops the current file, cleans up its temporary files and the server job, and skips the rest.
    The page goes back to Select source (path kept) with "Directory import cancelled: s of n files saved.".
    Tracks already saved stay.
  - **At the end** comes a summary: "Imported s of n files from <dir>.", the saved tracks with their dropped
    stems, and the failed files with their reasons. Show in Library selects the first saved track; "Import
    another track" is also there.
  - **Logs:** every started, saved, failed and cancelled file is written to the log (`calliope: import batch
    ...`).

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| Typed file → Browse opens its folder | `ac_browse_opens_the_typed_files_folder`; `browse_start_rules`; GUI `import_browse_start_folders` | PASS |
| Typed directory → Browse opens it | `ac_browse_opens_the_typed_directory`; `browse_start_rules` | PASS |
| Empty or missing path → the last folder, also after a restart | `ac_browse_opens_the_last_location_also_after_restart`; `pick_start_dir_rules` | PASS |
| Nothing imported, empty box → home | `ac_browse_opens_home_when_nothing_was_imported` | PASS |
| Local Directory: Browse opens the folder dialog and fills the box | `ac_directory_browse_uses_the_folder_dialog`; `ImportView.test.ts` | PASS |
| `b.mp3`, `a.flac`, `cover.jpg`, `sub/c.mp3` → `a.flac` then `b.mp3`, others ignored | `ac_directory_import_order_and_ignored_entries`; `list_audio_files_filters_and_sorts`; GUI `import_directory_batch` | PASS |
| No audio files → message, nothing starts | `ac_directory_without_audio_files`; `ac_directory_rejections_do_not_touch_the_remembered_folder` | PASS |
| 3 files, the second fails → first and third saved, summary lists the second | `ac_second_of_three_fails`; `batch_failures_go_on`; GUI `import_directory_one_fails` | PASS |
| Cancel on file 2 of 3 → first kept, rest not imported, idle page | `ac_cancel_on_file_two_of_three`; `ac_cancel_on_the_last_file_and_busy_refusal`; GUI `import_directory_cancel` | PASS |
| "File N of M: name" with the current file's progress | `ac_progress_names_the_current_file`; `ImportView.test.ts`; GUI `import_directory_batch` | PASS |
| Req. 1-2: `~`/relative paths, deleted last folder forgotten | `ac_browse_tilde_relative_and_deleted_last_folder` | PASS |
| Req. 4: spaces, uppercase extensions, hidden files | `ac_directory_with_spaces_uppercase_extensions_and_hidden_files` | PASS |
| Req. 10: log lines | `ac_batch_log_lines_for_started_saved_failed_cancelled` | PASS |

The headless acceptance tests are in `src/calliope-gui/tests/acceptance_existing_track_import.rs`. They run
against the stub `calliope-stems` on 127.0.0.1, whose test-only separator gained a `seq:` mode so that one file
in a batch can fail or hang. The unit tests are in `src/import_job_tests.rs`, `src/ipc.rs`, `src/local_source.rs`,
`src/picker.rs`, `src/settings.rs`, `src/ui/lib/import-state.test.ts` and `src/ui/views/ImportView.test.ts`. The
GUI tests are in `tests/gui_import_e2e.rs`.

## Task log

| Step | By | Result |
|---|---|---|
| plan | architect | 13 tasks (5 routine); your answers: remember the directory itself, skip hidden files, back to Select source on Cancel |
| task 1: stub separator `seq:` mode | **local (gemma4)** + me | correct; I put back a blank line it removed under a README table |
| task 2: `last_import_dir` setting | **local (gemma4)** + me | correct, but it replaced an existing test with the new one; I restored it |
| task 3: picker directory kind | implementer | the local model's edit deleted tests and broke the file; discarded |
| task 4: start folder rules, folder listing | implementer | done |
| tasks 5-6: shared import steps, batch runner | implementer | done; single imports behave as before |
| task 7: app commands | implementer | done |
| task 8: TS types | implementer | the local model deleted 128 lines of `ipc.ts`; discarded |
| tasks 9-10: import state, screens | implementer | done |
| task 11: acceptance tests | implementer | done |
| task 12: GUI tests, screenshots | implementer | done; I corrected the plan's remembered-folder line to the directory itself |
| task 13: README, `docs/ui.md` | implementer | moved from the local model after its failures |
| tester | tester | all criteria PASS; added 5 edge-case tests; no defects |
| reviewer | reviewer | APPROVED; 3 minor, 2 nits |

## Findings

- **Reviewer: APPROVED**, no blockers or majors. Left as follow-ups:
  1. *minor:* there is no limit on the number of files. A folder with tens of thousands of audio files would
     make a very large progress list and event. That is your own folder, so it is a size issue, not a
     security one.
  2. *minor:* for a moment between two files, the backend's progress state still shows the previous file's
     phase. Reopening the Import view in that moment shows it until the next event.
  3. *minor:* the frontend doesn't reset the per-file progress numbers when the next file starts. They are
     overwritten by the next events; a stale percentage could flash only if a file fails before the server
     reports work.
  4. *nit:* the batch log lines print file names as they are, so a file name with a newline would split a log
     line.
  5. *nit:* the folder is remembered only after the batch has started; if saving the settings fails, it is only
     logged, as for single imports.
- **Seen in the screenshots:** the "Done" heading on the progress page is shown before the first file finishes.
  Hiding it until the first file is done is a one-line change.
- **GUI suite:** `qa_malicious_server_stem_names_are_refused_in_the_ui` (an older stem-extraction test) timed out
  while the app was still starting, during the parallel run. It passes alone. A different test stalled the same
  way in the last build, so it looks like a start-up stall under load, not this feature.

## Deviations from the plan

- After a directory import, the remembered folder is the directory itself (your answer) instead of the plan's
  recommended parent folder.
- The GUI progress screenshot uses the stub mode `seq:slow,ok`, because the normal working phase is too short to
  catch.
- The local model did tasks 1 and 2, both with a fix-up by me. It broke tasks 3 and 8, so I moved those and task
  13 to the implementer. I've noted in memory that it can't be trusted with edits inside long files.

## Try it

`cd src/calliope-gui && npm run dev:app`, or `npm run build:app` and run `target/release/calliope-gui`. Go to
Import > Stem Extraction and choose Local Audio File, Local Video File or Local Directory.

## Deployment

The laptop app only: build with `npm run build:app` and install `target/release/calliope-gui` as before. Nothing
changes on archserver. `settings.json` gains the optional `last_import_dir` field, which older builds ignore.

## Manual checks (laptop, real dialogs and the real server)

- [ ] **M1, typed file.** Local Audio File, type the full path of a song in `~/Music/<band>/<album>/`, press
  Browse: the system dialog opens in that album folder.
- [ ] **M2, typed folder and invalid text.** Type `~/Music/<band>` and press Browse: the dialog opens there. Type
  `~/nothing-here`: no error, and the dialog opens in your home folder (or the last folder once you've imported
  something).
- [ ] **M3, after a restart.** Import one song from an album folder and quit. Start again, choose Local Video File
  with an empty box and press Browse: the dialog opens in that album folder.
- [ ] **M4, folder dialog.** Local Directory, Browse: a *folder* chooser opens in the last folder. Pick an album
  folder: its path fills the box.
- [ ] **M5, a real batch.** With the real server set in Settings, Extract an album folder with 3-4 songs (plus a
  cover image or a subfolder):
  - the page shows "File 1 of N: ..." with Working... progress, and the list of files done grows;
  - each song becomes its own stem track with its tags;
  - the cover and the songs in the subfolder are not imported;
  - the summary lists the tracks and dropped stems, and Show in Library opens the first one.
- [ ] **M6, a failing file.** Add a text file renamed to `broken.mp3` to a copy of the folder and run it again:
  the summary lists `broken.mp3` with its reason, and the other songs are saved.
- [ ] **M7, Cancel.** Start a batch of 3+ songs and press Cancel while the second is "Working...":
  - the page returns to Select source with "Directory import cancelled: 1 of N files saved.";
  - the first track is in the Library;
  - the server's journal (`journalctl --user -u calliope-stems` on archserver) shows the job deleted.
- [ ] **M8, a folder without songs.** Extract a folder that holds only images: "Directory ... has no audio files"
  appears under the box, and nothing starts.

Once you've verified it, tick `gui-import-local-batch.md` in the roadmap in `specs/overview.md`.
