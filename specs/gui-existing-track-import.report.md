# Report: import a track from a local file by path

**Status: DONE.** Requirements 1-3 and both acceptance criteria pass. The reviewer approved (and
approved the fix round), and the tester's two defects were fixed in one round. Nothing touched
archserver, a sound device or the calliope-stems server running on this laptop.

- Branch: `spec/gui-existing-track-import` (base `09d92c2`), not pushed or merged
- Plan: `specs/gui-existing-track-import.plan.md`, log: `specs/gui-existing-track-import.log.md`
- **Nothing to deploy on archserver:** only the desktop app changed.

## What changed for you
Local audio and video import already existed (from gui-stem-extraction, with stem extraction, as
you intended). This build adds what the spec asks on top:
- Import > Stem Extraction > **Local Audio File** or **Local Video File** now shows a **path box**
  with **Browse** and **Extract** on one row.
- Type or paste a path: `/…`, `~/…`, a path in quotes, or a `file://` link (what a file manager
  copies). Extract or Enter starts the same stem-extraction import as before.
- **Browse now only fills the box** (and focuses Extract, so Enter imports). Before, Browse
  started the import straight away.
- A path that can't be used gets an exact message under the box, and nothing starts:
  `No file found at …`, `… is a folder, not a file`, `… is not a regular file`, `Cannot read …`,
  `"x.mp3" is not a video file` (or audio), `Enter the full path, starting with / or ~/`,
  `The path is not valid`, and for an empty box `Enter the path of a file or choose one with Browse`.
- Switching between audio and video clears the box. A video's first audio track is used (your
  decision).

## Acceptance criteria

| Criterion / requirement | Tests | Result |
|---|---|---|
| Req 1: audio → path box + Browse (system dialog) | vitest path-row and Browse cases; tester `req 1 and 2: path box + Browse per kind`, `Browse (%s) only fills the box`; GUI `import_browse_fills_path`, `import_local_audio` | PASS |
| Req 2: video → path box + Browse | the same, video kind; GUI `import_local_video`, `import_typed_video_path` | PASS |
| Req 3: audio extracted from the video, then imported like audio | `a_video_with_two_audio_streams_imports_the_first_one`, `qa_req3_video_without_audio_and_first_stream`, existing video-flow tests | PASS |
| AC1: audio + valid path → stems in the library | `typed_path_forms_import_audio_and_video` (plain, quoted, `file://`), `qa_ac1_ac2_home_tilde_and_symlink_paths_import_stems` (`~/`, quotes, symlink); GUI `import_typed_audio_path` | PASS |
| AC2: video + valid path → stems of its audio in the library | the same tests, mp4; GUI `import_typed_video_path` | PASS |
| Invalid paths: exact message, no job, nothing written | `local_source` unit tests, `rejected_paths_start_no_job_and_create_nothing`, `qa_exact_rejection_messages`, `qa_security_fifo_dev_and_hostile_inputs_do_not_hang_or_write`; GUI `import_typed_path_errors` | PASS |
| No regression in the URL import | vitest `URL import still works and sends no path`; GUI URL tests | PASS |

Suites:
- **Headless** (`npm test`, display unset): green after the fix round (vitest 406 tests; Rust
  workspace 0 failed; clippy clean).
- **GUI** (`DISPLAY=:1 npm run test:gui`): 39 tests in six suites, green (run twice by the
  implementer, once by the tester).
- Screenshots `target/gui-shots/import-path-*.png`: the box, Browse and Extract on one row; the
  error in the accent colour directly under it; Extract focused after Browse.

## Design
- `src/local_source.rs` (new, pure): cleans up a typed path (quotes, `~/`, `file://` decoding)
  and checks it in a fixed order: absolute, exists, not a folder, a regular file (so a FIFO or
  device is refused before anything opens it), readable, right extension.
- Commands: `pick_import_file(kind)` opens the dialog and returns `{path, name}`;
  `import_path(kind, path)` checks the path and starts the existing import. `import_file` is gone.
- **Architecture exception** (recorded in `docs/architecture.md`): the frontend now sends a path.
  `import_path` only reads the file (ffprobe/ffmpeg on it), so it can do no more than Browse.

## Task log

| Task | Done by |
|---|---|
| 1. Path clean-up and checks | implementer |
| 2. `pick_import_file` and `import_path` | implementer |
| 3. Frontend wrappers | **local model** (correct code) + implementer (two test files the plan missed) |
| 4. The path row on the source page | implementer |
| 5. Headless acceptance | implementer |
| 6. GUI tests and screenshots | implementer |
| 7. README | implementer (the local model pasted its instructions verbatim; reverted) |
| tester: both criteria pass; 2 defects | tester |
| reviewer: APPROVED | reviewer |
| fix round 1 | implementer; reviewer APPROVED |

**Local model (qwen3-coder:30b, new setup):** task 3 was its first usable result: the code was
right, but it claimed the tests passed when 5 were failing. Task 7 failed. It is useful for exact
code edits; its own reports can't be trusted, and every result needs checking.

**Fix round:** the README had a non-ASCII "…" (introduced by the orchestrator in task 7) that
broke an existing test; and a `file://` link with an encoded control character (`%00`, `%0A`)
got a garbled "Cannot read" message instead of "The path is not valid". Both fixed, plus two
reviewer minors: `file://` links with `?` or `#` are rejected, and the check-then-open window
is noted in the architecture doc.

## Deviations
- Task 3's file list missed two test files that mocked the removed command; they were updated in
  the same task.

## Remaining minors (optional)
- The empty-box message exists in both Rust and TypeScript and could drift.
- No unit test pins that a plain path containing `#` or `?` (e.g. `/music/a#1.mp3`) stays valid;
  the code handles it correctly.
- Between the check and ffprobe opening the file, a file swapped for a FIFO could block ffprobe;
  Cancel still works.

## Follow-up idea
- Videos with several audio tracks: choose the track, or one stem track per audio track (you chose
  "first track only" for now).

## How to try it
```sh
cd src/calliope-gui && npm run build:app && ../../target/release/calliope-gui 2>&1 | grep calliope
```

## Manual checks (laptop)
- [ ] Import > Stem Extraction > Local Audio File: the path box, Browse and Extract are on one row
- [ ] Browse a real mp3: the box shows its full path and nothing starts; Enter opens the edit pane
  with the tags filled in; Extract puts the stems in the Library
- [ ] Copy a file in your file manager (Ctrl+C) and paste it into the box; also type
  `~/Music/<file>` by hand. Both import
- [ ] Type a path with a typo: "No file found at …" appears under the box; fix it and continue
- [ ] Local Video File: paste a real video's path: edit pane, then stems of its audio. With a video
  that has several audio tracks, the first one is used
- [ ] A silent video gives "Selected file <name> has no audio track"
- [ ] Keyboard only: Tab reaches the box, Browse and Extract in that order; Enter in the box imports

Once verified, tick `gui-existing-track-import.md` in the roadmap of `specs/overview.md`.
