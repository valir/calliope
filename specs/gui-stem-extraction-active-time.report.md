# Report: empty stems are judged by audible time

**Status: DONE.** The rewritten requirement 9 and its three criteria pass. The reviewer approved
(and approved the fix round), the tester found no production defects, and one fix round removed
four timing flakes from the tests. Nothing touched archserver, the real model, a sound device or
the calliope-stems server running on this laptop.

- Branch: `spec/gui-stem-extraction-active-time` (base `c969054`, on top of
  `spec/gui-stem-extraction-server-peaks`), not pushed or merged
- Plan: `specs/gui-stem-extraction-active-time.plan.md`, log: `specs/gui-stem-extraction-active-time.log.md`
- **Deployment needed:** `calliope-stems` on archserver (steps below). The new app works with
  every server version, so the order doesn't matter.

## The rule
A stem is **empty** when it is audible for less than 15 s in total: the song is cut into 100 ms
windows, and a window is audible when its level (RMS, loudest channel) is above -40 dBFS. Peak no
longer matters: on your two tracks the empty stems peak as high as -8.7 dBFS, while they are
audible for only 0.2-10.8 s; real parts are audible for 200-362 s.

Measured with the new `calliope-stems --measure` on your existing tracks (identical to the
orchestrator's independent measurement):

| Track | Kept | Dropped |
|---|---|---|
| 2 Minutes to Midnight | guitar 361.6 s, bass 343.8 s, drums 343.1 s, vocals 200.6 s | other 10.8 s, piano 1.8 s |
| Losfer Words | drums 252.7 s, guitar 250.4 s, bass 245.6 s | other 9.1 s, vocals 6.3 s, piano 0.2 s |

## Acceptance criteria

| Criterion / decision | Tests | Result |
|---|---|---|
| Empty = audible < 15 s in 100 ms windows above -40 dBFS, loudest channel | `flac_level` unit tests (327/328 amplitude, 149/150 windows, partial window, stereo, 8/24-bit full scale); tester `measure_agrees_with_an_independent_oracle_on_boundaries_and_odd_layouts` (ffmpeg-based oracle sharing no code) | PASS |
| Stems audible < 15 s are dropped even with short loud artifacts | `bursts.flac` (peak -8 dBFS, 9.5 s) in unit, import and GUI tests; `a_part_with_pauses_and_a_loud_peak_artifact_stem_are_judged_by_time_not_peak` | PASS |
| A real part with pauses adding up to ≥ 15 s is kept | `phrases.flac` (8 × 2 s); `audible-15000ms` in three pieces | PASS |
| 14.9 s dropped, 15.0 s kept, via the server report and via the app's own check | `boundary_14_9_vs_15_0_via_the_local_check_and_via_a_server_report`, import_job `boundary` tests | PASS |
| Server-reported empty stems are not downloaded and are listed as dropped | server request logs in import_job, acceptance and GUI tests (`import_drops_empty_stems`, `import_boundary_stems`) | PASS |
| Server and app agree bit for bit | `server_report_equals_the_standalone_measure_of_the_same_stems_bit_for_bit`, `real_server_with_and_without_levels_gives_the_same_track_and_the_same_numbers` | PASS |
| Old server, server-peaks-era server, report at another level/window: everything fetched, same verdict | `identical_verdict_for_every_kind_of_server_only_the_download_list_differs` (5 server kinds) | PASS |
| All empty: "Every stem is empty (less than 15 s above -40 dBFS), so no track was saved", audio kept for retry | `all_empty_fails_with_the_exact_message_for_every_kind_of_server_and_keeps_audio_for_retry` | PASS |
| `calliope-stems --measure` | 6 tester tests (lines, `-inf` peak, errors per file with exit 1, no `--separator` needed, never listens) | PASS |
| UI says "Dropped empty stems: …" | vitest; GUI screenshots `target/gui-shots/import-done-dropped.png`, `import-done-boundary.png` | PASS |

Suites:
- **Headless** (`npm test`, display unset): 2476 passed, 0 failed; clippy clean.
- **GUI** (`DISPLAY=:1 npm run test:gui`): all six suites green (4 + 5 + 2 + 15 + 8 + 1).

## Design
- **One shared measure:** `calliope_lib::flac_level` (claxon, integer sum of squares compared with
  an exact integer limit), used by the server and the app. It replaces `flac_peak`.
- **Protocol (still API v1, additive):** a done job carries
  `stem_levels: [{name, audible_ms, level_dbfs, window_ms, peak_dbfs}]`, replacing `stem_peaks`.
  The 15 s limit lives only in the app, which uses a report only if it was counted at its own
  level (-40) and window (100 ms). An older server (no key, or `stem_peaks` only) means the app
  downloads and checks every stem itself. The list is validated only on done jobs.
- **Server:** measures every stem fully after separation (cancel/shutdown checked between stems);
  logs `stem=… audible_ms=… windows=… level_dbfs=-40 peak_dbfs=…` and `measured=n/total ms=…`.
  `--no-stem-levels` replaces `--no-stem-peaks`. New: `--measure FILE...`.
- **App:** server-reported empty stems are not fetched; every fetched stem is still checked locally
  (stopping once 15 s is proven). Logs show `audible_s`, `source=server|local` and
  `server_levels=n/total`. The finished page says "Dropped empty stems: …".
- **Test stems:** the canned stems are now 16 s long (under the new rule the old 1 s ones would all
  be empty); new `stems-activity/` fixtures (bursts, phrases, 14.9/15.0 s boundary); stub modes
  `sparse` (changed contents, same kept/dropped sets) and `boundary` (new).

## Task log

| Task | Done by |
|---|---|
| 1. 16 s canned stems, activity stems | implementer (the local model ignored the task twice) |
| 2. Stub separator mode `boundary` | implementer |
| 3. `calliope_lib::flac_level` | implementer |
| 4. `stem_levels` in the protocol | implementer |
| 5. The server measures audible time, `--no-stem-levels` | implementer |
| 6. `calliope-stems --measure` | implementer |
| 7. The app judges by audible time (peak-rule tests ported or deleted) | implementer |
| 8. Frontend wording | implementer (the local model wrote a summary instead of edits) |
| 9. Remove `stem_peaks` and `flac_peak` | implementer |
| 10. GUI tests | implementer |
| tester: 22 acceptance checks, all pass | tester |
| reviewer: APPROVED | reviewer |
| fix round 1: test-harness races, full-scale level tests | implementer; reviewer APPROVED |

**The local model handled none of its four routine tasks.** Each time it answered the session's
commit-attribution notice or wrote a conversation summary and made no edits. That looks like a
`local-claude` problem (the instructions it receives drown the task), worth looking into before
the next build.

**Fix round:** four tests had each failed once during the build. The tester traced all of them to
test timing, not product bugs: a fast stub can skip the "Working" phase; the job snapshot goes idle
a moment before its last event arrives; a submit after aborted uploads can briefly find the queue
full. The helpers now wait for the terminal event and retry a full queue; each fixed test then
passed 20/20 under CPU load.

## Deviations
- Stems are dropped in server order (`piano, other`); the plan text said otherwise and was corrected.
- 32-bit FLAC still can't be decoded (claxon), so the 32-bit case is tested on the arithmetic only,
  and a 32-bit stem is kept. Demucs writes 16- or 24-bit.
- The tests that pinned the -50 dBFS peak rule were deleted or ported; one was inverted (a 2 s loud
  passage in a long quiet stem is now empty, as the new rule intends).

## Remaining minors (optional)
- **A song shorter than 15 s** has every stem empty, so its import always fails with "Every stem is
  empty". That follows from requirement 9 as written; say if you want a different rule for very
  short songs.
- A cancel while the server measures takes effect between stems (a few seconds at most).
- A test comment in `flac_level.rs` (8-bit case) is muddled; the assertions are right.

## Deployment (archserver, by you)
1. Check out this branch (or `main` once merged) on archserver.
2. **Check the unit file first:** if `~/.config/systemd/user/calliope-stems.service` still has
   `--no-stem-peaks` in `ExecStart`, remove it and run `systemctl --user daemon-reload`. That flag
   no longer exists, and the server refuses to start with an unknown flag.
3. From the repository top: `bash script/calliope-stems-deploy.sh` (builds, installs, restarts, shows
   `listening`).
4. `calliope-stems --help | grep -e --no-stem-levels -e --measure` shows both options.

Laptop: `cd src/calliope-gui && npm run build:app` as usual.

## Manual checks
- [ ] Deploy the server, then re-import **2 Minutes to Midnight**
  (`https://www.youtube.com/watch?v=YCmUqAffWS8`). The finished page says "Dropped empty stems:
  piano, other"
- [ ] The journal on archserver has one `stem=… audible_ms=…` line per stem, `measured=6/6 ms=…`
  (note the `ms`), and no `…/stems/piano` or `…/stems/other` request
- [ ] The app's output (`grep "calliope: import"`) shows `server_levels=6/6` and `source=server`
  for the dropped stems
- [ ] The same for **Losfer Words** (`https://www.youtube.com/watch?v=7mMqOmeyzPU`): vocals, piano
  and other dropped
- [ ] Both new tracks open in the Editor and play, and nothing musical is missing. The old copies
  (`01a124a2-…`, `01a124b2-…`) are untouched; delete them from the Library if you like
- [ ] Optional fallback: add `--no-stem-levels` to `ExecStart`, `daemon-reload`, restart, import
  again: `server_levels=0/6`, every stem downloaded, the same stems dropped with `source=local`.
  Remove the flag afterwards

Merge order: `spec/gui-stem-extraction-clear-empty`, then `spec/gui-stem-extraction-server-peaks`,
then this branch (or just this branch, which contains both). Once verified, update
**gui-stem-extraction** in the roadmap of `specs/overview.md` as you see fit.
