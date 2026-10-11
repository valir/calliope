# Report: stems beat autodetection

**Status: DONE.** All 17 accepted criteria (A1-A17, plan section 7) pass. The reviewer approved twice, the second
time after the fix round, and the tester confirmed the fixes with no new defects.

- **Branch:** `spec/stems-beat-autodetect` (base `f03dcfd`), not pushed or merged.
- **Plan:** `specs/stems-beat-autodetect.plan.md` (revision 2). **Log:** `specs/stems-beat-autodetect.log.md`.
- **Deployment on archserver is needed** for real detection: see "Deployment" below. Until then, Autodetect says the
  server has no beat detection, and stem separation keeps working.

## What you get
- **Editor, Mix lane:**
  - **Waveform:** shows the current mix, built from the checked stems at their volumes. It redraws 0.3 s after the
    last change, and each new change restarts that wait. It is computed on its own thread, so playback and the
    sliders never wait for it.
  - **Autodetect BPM:** renders the mix, sends it to calliope-stems ("Beat This!"), and turns the beats into tempo
    sections. While it runs, the button reads **Cancel** and progress text appears in the message line.
  - **When Autodetect is enabled:** at least one checked stem above Off, and an edge-AI server set in Settings.
  - **Errors** are shown in the message line. The cases are: server unreachable, server without beat detection,
    tracker failure, too few beats, and a `player.json` written by a newer version.
- **Sections on the waveform:** the first BPM is at the left edge, with no bar. Each change is an amber bar followed
  by its BPM, for example "90 BPM". Sections can be 1 bar long, including a last bar at the very end of a song.
- **Fine adjustment:** a detected change moves in 1/64-beat steps of the section before it.
  - **Limit:** at most ±4 beats (±256 steps) in total from where it was detected, however many moves you make. A
    change always stays at least one beat from its neighbours.
  - **By mouse:** drag the bar. A readout shows the offset, for example "+160/64 beat (+0:01.250) · limit ±4 beats".
  - **By keyboard:** focus the waveform, then ↑/↓ chooses a change, ←/→ moves one step and Shift+←/→ moves one beat.
  - **During detection:** moves are blocked.
- **Storage:** the tempo map lives in the track's `player.json`, now schema 2. Each detected change keeps its
  detected position. A detected change starts its section on a beat, so the last beat before it is stretched or
  shortened a little, and the metronome follows it.
- **Player:**
  - **Sections:** shows the same sections, read-only.
  - **Detect button:** removed.
  - **When the Editor saves a new tempo map:** the Player first saves its loops, position and settings, then closes
    the track. It reopens it from `player.json` at once if it is on screen, otherwise the next time it is.
- **Editor tabs:** the "BPM & sections" tab is removed.
- **calliope-stems:**
  - **New endpoints:** `POST /v1/beats` and `GET /v1/jobs/<id>/beats`.
  - **New flags:** `--beat-tracker`, plus a beat timeout.
  - **Health:** lists `"tasks"`.
  - **Missing tracker:** a missing tracker path only gives a warning, and the server runs with stems only.

## Acceptance criteria

| Criterion | Tests | Result |
|---|---|---|
| A1 mix waveform with playhead | GUI `mix_waveform_and_autodetect`, screenshots | PASS |
| A2 0.3 s debounce, non-blocking | `a2_a3_waveform_recomputes_once_300ms_after_…`, `a2_debounce_…`, editor unit tests with a blocked worker | PASS |
| A3 a new change restarts the wait, one recomputation | same | PASS |
| A4 Autodetect enabled only with a checked stem above Off and a server | vitest A4, GUI | PASS |
| A5 mix rendered, sent to the server, phases, Cancel | `a5_a8_a9_a10_…`, GUI | PASS |
| A6 cancel, track switch, app exit leave nothing behind | `a6_cancel_close_and_exit_…`, detection racing a Save, GUI | PASS |
| A7 failures leave the tempo map unchanged | 13 hostile beat payloads, no-beats server, unreachable server, too few beats, newer `player.json` | PASS |
| A8 a steady song gives one tempo | `a5_a6_a8_a10_final_one_bar_section_and_steady_song` | PASS |
| A9 several tempos, a final 1-bar section, changes on a beat | `a9_a12_detected_changes_start_on_a_beat_…`, erratic beat lists | PASS |
| A10 saved in `player.json` schema 2, loops and position kept | `a5_a8_a9_a10_…` | PASS |
| A11 the Player saves, closes and reloads | GUI `autodetect_errors_cancel_and_player_reload`, unit tests | PASS |
| A12 1/64-beat steps, ±256 in total, one beat from neighbours | `a12_*` | PASS |
| A13 keyboard alternative | `WaveformView` tests, GUI keys up to 256 | PASS |
| A14 no Detect button in the Player | vitest | PASS |
| A15 typed BPM and detected positions | `typing_bpm_zero_…`, `typed_bpm_inside_a_stretched_beat_…`, `fix1_*` | PASS |
| A16 the Player shows sections, read-only | screenshot, `WaveformView` read-only test | PASS |
| A17 Editor tabs: Stems, Tablature, MIDI cues | vitest, screenshot | PASS |

**Suites:**
- **Headless** (`env -u DISPLAY npm test`): green. vitest ran 541 tests. Rust ran 976 passed, 0 failed. Clippy is
  clean.
- **GUI** (`npm run test:gui`, parallel): 49 tests in 143 s, green.

## Task log

| Task | Done by |
|---|---|
| 1-5 Server side: client, stand-in tracker, beat jobs, tests, Beat This! adapter, licences | implementer |
| 6-8 Tempo map with detected changes, schema 2, beats → sections | implementer |
| 9-13 Mix waveform, worker, tempo store and Player hand-over, detection job, IPC | implementer |
| 14 Remove the Player's Detect button | **local model** (one test title missed; fixed by the orchestrator) |
| 15 Remove the "BPM & sections" tab | **local model** (diff correct; the plan missed `App.test.ts`, which was fixed in a follow-up commit) |
| 16-20 Geometry, shared waveform, Editor state, Mix lane, GUI e2e + visual review + docs | implementer |
| tester | A1-A17 PASS; 2 defects |
| reviewer | APPROVED, 9 minors |
| fix round 1 | implementer; reviewer APPROVED, tester confirmed |

**Local model** (qwen3-coder:30b): both routine tasks were exact edits to existing files, and both were essentially
right. That is its best result so far. Qwen 2.5 Coder 14B was tried and can't make tool calls with Claude Code.

**Found and fixed during the build:**
- **Task 12, half-time ending:** a half-time last bar at the end of a song was read as 66.7 BPM instead of 60. The
  segmentation now handles a final bar of 4 beats with nothing after it.
- **Fix round 1:**
  - **Crash on extreme input:** an extreme move value from a malformed request crashed a debug build. It is now
    refused.
  - **BPM typed inside the stretched beat:** a BPM typed inside the stretched last beat before a detected change
    could leave the beat grid out of order (92 of 5,880 cases). That BPM now applies at the detected change.
  - **Service unit:** the example unit would have stopped the whole stems server if the adapter wasn't installed.
    The server now only warns.
  - **Moves during detection:** refused.
  - **Release check:** the Player is no longer closed for a `player.json` that can't be written.
  - **Uploaded audio:** a finished beat job's uploaded audio is deleted.
  - **Two small robustness fixes:** in the waveform drag and the first waveform computation.

## Deviations from the plan
- **Snapping detected changes:** your "1/64 steps" became 1/64 of a beat, so 256 steps make 4 beats. Changes are
  stored in ms with their detected position. The Player's loop grid is unchanged.
- **Section detection:** a new segmentation replaces the plan's 8-beat rule, so 1-bar sections are found.
- **Test fixtures:** the e2e tests use the 12 s "Four Lanes" fixture, so the stand-in's change is at 4500 ms, not at
  6500 ms.
- **Player layout:** with the Detect button gone, the Playback group moved up beside BPM and Metronome.
- **Editor layout:** at 1024x640 the Editor scrolls, as it did before.

## Known limits and remaining minors (optional)
- **Merging equal tempos:** typing, in the Player, the same BPM as a neighbouring section merges the two sections. If
  you do it just before a detected change, that change disappears. This is the existing "equal neighbours merge"
  rule.
- **Loop drift:** many fine moves can shift a loop by up to 1/32 beat, because loops are re-snapped to ticks each
  time.
- **Glitchy beat lists:** cleaning up a very glitchy beat list is slow in theory (O(n·k)). It is capped at 20,000
  beats and runs off the UI thread.
- **Label overlap:** the "120 BPM" label can touch the playhead line.
- **Detection accuracy:** a 1-bar section only 4-5 % off is found only when Beat This! jitters by about 10 ms or less.
  Half-time and double-time endings are always found.
- **Licence rows:** the Python package licences were checked on PyPI, not in the installed metadata (deployment step
  7).

## Deployment (archserver, owner only)
1. `git pull && cargo build --release -p calliope-stems`
2. Create the Beat This! environment and download the checkpoint (about 4 GB, mostly torch):
   ```bash
   uv venv --python 3.12 ~/edge-ai/beats/.venv
   uv pip install --python ~/edge-ai/beats/.venv/bin/python \
       --index-url https://download.pytorch.org/whl/cu128 torch==2.11.0 torchaudio==2.11.0
   uv pip install --python ~/edge-ai/beats/.venv/bin/python beat-this==1.1.0 soundfile
   mkdir -p ~/edge-ai/beats/models
   curl -fL -o ~/edge-ai/beats/models/final0.ckpt \
       https://cloud.cp.jku.at/public.php/dav/files/7ik4RrBKTS273gp/final0.ckpt
   ```
3. Install the binaries:
   ```bash
   install -m755 target/release/calliope-stems ~/.local/bin/
   install -m755 src/calliope-stems/trackers/beat-this.sh ~/.local/bin/calliope-beats-beat-this
   ```
4. Smoke test: `BEATS_HOME=~/edge-ai/beats calliope-beats-beat-this song.flac /tmp/bt && head -c 300 /tmp/bt/beats.json`
5. Edit the unit `~/.config/systemd/user/calliope-stems.service`:
   - add `Environment=BEATS_HOME=%h/edge-ai/beats`;
   - add `--beat-tracker %h/.local/bin/calliope-beats-beat-this` to `ExecStart`, as in
     `src/calliope-stems/deploy/calliope-stems.service`;
   - then run `systemctl --user daemon-reload && systemctl --user restart calliope-stems`.
6. From the laptop, `curl http://archserver:8765/v1/health` shows `"tasks":["stems","beats"]`.
7. Optional: compare the installed licences (`.dist-info/METADATA`) with `docs/licences.md`.

On the laptop, build with `cd src/calliope-gui && npm run build:app`. The app now writes `player.json` schema 2, and
older builds open such files read-only.

## Manual checks (laptop + archserver)
- [ ] **M1 waveform:** on a real stem track, check drums, bass and vocals. The waveform appears within about 1 s.
  Unchecking a stem redraws it about 0.3 s after the last click, and playback never stutters.
- [ ] **M2 steady song:** run Autodetect BPM. The status goes from preparing through uploading and detecting to
  saving, then shows the result line. In the Player, turn the Metronome on and play: the clicks stay on the beat to
  the end.
- [ ] **M3 tempo change:** a song with a tempo change gets an amber bar near the change with the new BPM, in both the
  Editor and the Player. In the Player, V is ticked and the metronome follows the change.
- [ ] **M4 1-bar ending:** a song ending with a 1-bar change, for example a half-time last bar, shows a separate
  section.
- [ ] **M5 fine adjustment:** drag a change bar. It stops 4 beats from where it was detected, also across several
  drags. Click the waveform, then ←/→ moves 1/64 beat and Shift+←/→ moves a beat. In the Player, the first click of
  the new section lands on the moved bar.
- [ ] **M6 Player hand-over:** in the Player, add a loop and pause mid-song. Switch to the Editor and run Autodetect.
  Back in the Player, the track has reloaded at the same position, the loop is on the same music, and the new tempo
  shows.
- [ ] **M7 cancel and server errors:** start Autodetect and Cancel at once. The status shows cancelled, the server
  journal shows the job cancelled, and no `python` process is left. With the server stopped, the message says it
  can't be reached.
- [ ] **M8 detection time:** note how long a 4-minute song takes. Optionally check `nvidia-smi` to see whether it ran
  on the GPU or fell back to the CPU.
- [ ] **M9 removed controls:** the Player has no Detect button, and the Editor has no "BPM & sections" tab.

Once you have verified it, add or tick `stems-beat-autodetect.md` in the roadmap of `specs/overview.md`.
