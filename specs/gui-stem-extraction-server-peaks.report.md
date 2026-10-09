# Report: the server reports stem peaks; silent stems are not downloaded

**Status: DONE.** Requirement 9 and its acceptance criterion pass. The reviewer approved, the
tester found no defects, and one small fix round handled a test-harness race and a docs note.
Nothing touched archserver's running service, the real model or a sound device.

- Branch: `spec/gui-stem-extraction-server-peaks` (base `f62ecf9`, on top of
  `spec/gui-stem-extraction-clear-empty`), not pushed or merged
- Plan: `specs/gui-stem-extraction-server-peaks.plan.md`, log: `specs/gui-stem-extraction-server-peaks.log.md`
- **Deployment needed:** `calliope-stems` on archserver (steps below). The new app also works with
  the old server, so the update order doesn't matter.

## Acceptance criteria

| Criterion / decision | Tests | Result |
|---|---|---|
| The server reports each stem's peak when a job is done (`stem_peaks`; key absent before `done`, on failed/cancelled jobs and with `--no-stem-peaks`) | stems `conformance` (3 new), `acceptance_server_peaks` (11) | PASS |
| Stems the server reports below -50 dBFS are not downloaded and are listed as dropped | `import_job_tests` `server_silent_stems_are_never_fetched`; GUI `import_drops_silent_stems` (request log: piano and other never fetched); `acceptance_server_peaks` GET counts | PASS |
| With a server that doesn't report peaks, every stem is downloaded and checked as before, same result | `an_old_server_without_peaks_gives_the_same_result_with_all_stems_fetched`; `old_server_without_the_key_downloads_and_checks_everything` | PASS |
| Same decision whoever measures (16-bit 103/104, 24-bit 26527/26528) | `decision_is_identical_for_server_and_local_peaks_at_the_boundaries` | PASS |
| An old app parses the new JSON | `json_is_still_parseable_by_an_old_shaped_client`, `stem_peaks_serde_and_compat` | PASS |
| Malformed or hostile peak lists are rejected before any download (15 variants) | `malformed_peak_lists_are_rejected_before_any_download`, `acceptance_client_hostile` | PASS |
| An unmeasurable stem has no entry; the app downloads and keeps it | `undecodable_stem_gets_no_entry_but_the_job_succeeds`, `real_server_undecodable_stem_is_kept_and_downloaded` | PASS |
| All stems silent: the import fails with no downloads at all | `all_silent_by_the_server_fetches_nothing`, `real_server_all_silent_has_zero_stem_downloads` | PASS |
| Cancel, delete or server shutdown while the server measures | `cancel_while_the_server_is_measuring_leaves_nothing`, `sigterm_during_measuring_exits_promptly`, `server_killed_while_measuring_fails_the_import_cleanly` | PASS |

Measured in the tests:
- **Measuring time:** 6 stems of 10 min stereo 44.1 kHz took 2.3 s on the server.
- **Interrupting a measurement:** a DELETE was answered in 0.75 s, a SIGTERM exited the server in
  1.2 s, and an app cancel stopped the import in 0.9 s.

Suites:
- **Headless** (`npm test`, display unset): all crates pass, 0 failed; clippy clean.
- **GUI** (`DISPLAY=:1 npm run test:gui`): all suites green, run twice.

## Design
- **One shared measurement:** `calliope_lib::flac_peak` (integer peak scan, `claxon`), used by the
  server and the app. The -50 dBFS threshold lives only in the app.
- **Protocol (still API v1, additive):**
  - When done, `GET /v1/jobs/<id>` carries `stem_peaks: [{name, peak, bits, peak_dbfs}]`. The app
    decides from the integer `peak` and `bits`; `peak_dbfs` is only for people.
  - Digital silence is `peak: 0`. A stem the server couldn't measure has no entry.
  - The key is left out when there are no peaks, so old apps are unaffected.
  - `--no-stem-peaks` turns measuring off and reproduces the old JSON exactly.
- **Server:** measures every stem after the separator finishes, outside the job lock, checking for
  cancel or shutdown between stems. It logs one line per stem, plus `measured=<n>/<total> ms=<ms>`.
- **App:**
  - Stems reported silent are not fetched; they still appear in "Dropped silent stems".
  - Every downloaded stem is still checked locally, and local silence wins.
  - Logs show `source=server|local` and `server_peaks=<n>/<total>`.
- **Trust:** the app drops stems it never downloads on the server's word. The architect accepted
  this because the server is LAN-only and already supplies the audio; it's recorded in
  `docs/architecture.md`.

## Task log

| Task | Done by |
|---|---|
| 1. Shared peak scan in `calliope-lib`, server licence notices | implementer |
| 2. The app's check uses the shared scan | implementer |
| 3. `stem_peaks` in the protocol, client validation | implementer |
| 4. Stub separator mode `undecodable` | local model; the orchestrator fixed a doubled backslash |
| 5. The server measures and reports, `--no-stem-peaks` | implementer |
| 6. The import skips stems the server reports silent | implementer |
| 7. GUI test: silent stems never fetched | implementer |
| tester: 23 acceptance checks, all pass | tester |
| reviewer: APPROVED | reviewer |
| fix round 1: test-harness race, trust note | implementer |

**Fix round:** the tester traced the occasional test failures seen over the last two increments
(`cancel_while_the_check_runs_leaves_nothing`, `a_video_without_audio_fails_with_the_exact_text`) to
a race in the tests' wait helpers, which could return a job state from just before it changed. The
helpers were fixed in three test harnesses (test code only). The affected suites then passed 5 times
in a row.

## Deviations
- **32-bit FLAC:** claxon can't decode it, so a 32-bit stem counts as undecodable: the server sends
  no entry for it, and the app keeps it. The 32-bit arithmetic is tested on numbers only. Demucs
  writes 16- or 24-bit, so this doesn't affect real imports.
- **Server shutdown during measuring** fails that job. The plan didn't specify this case; the jobs
  are in memory anyway, so nothing survives a restart.

## Remaining minors (optional)
- The app validates `stem_peaks` whatever the job state, so a failed job carrying garbage peaks
  would report "bad stem peak list" instead of its own error. A real server never sends that.
- The app's stall timer sees "running" with no progress while the server measures. This takes a
  few seconds at most for a 15-minute song, far below the timeout.

## Deployment (archserver, by you)
1. `git pull` (once merged) or check out this branch, then `cargo build --release -p calliope-stems`
2. `install -m755 target/release/calliope-stems ~/.local/bin/`. The separator script is unchanged.
3. `systemctl --user restart calliope-stems`. Jobs in progress are lost, as with any restart.
4. `journalctl --user -u calliope-stems -n 5` shows `listening`
5. `calliope-stems --licenses | grep -i claxon` lists claxon

## Manual checks
- [ ] After the redeploy, import a song with no piano. The journal shows one
  `stem=… peak=… bits=… peak_dbfs=…` line per stem and `measured=6/6 ms=…`; note the `ms`
- [ ] The app's output (`grep "calliope: import"`) shows `server_peaks=6/6`, and piano/other
  `dropped … source=server`. The journal has no `…/stems/piano` request, and "Import finished"
  lists the dropped stems
- [ ] The track folder holds only the kept stems; they open and play in the Editor
- [ ] Optional fallback: add `--no-stem-peaks` to the unit's `ExecStart`, `daemon-reload`,
  restart, and import again. You should see `server_peaks=0/6`, every stem downloaded, and the same
  stems dropped with `source=local`. Remove the flag afterwards
- [ ] An old app build, if you have one, still imports from the new server

Merge order: `spec/gui-stem-extraction-clear-empty` first, then this branch. Once verified, update
**gui-stem-extraction** in the roadmap of `specs/overview.md` as you see fit.
