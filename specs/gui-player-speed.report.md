# Report: Player speed and speed training

**Status: DONE.** All acceptance criteria pass. The reviewer approved after 2 fix rounds. The full headless suite
(633 frontend tests and all Rust tests) and the full GUI suite pass on `df94e62`. Branch `spec/gui-player-speed`,
base `e49615e`, not pushed or merged.

## What was built

- **Speed control** in the Playback pane: Speed, `-` (Slower), a % box, `+` (Faster), Reset and a read-only BPM
  readout.
  - The speed goes from 25 to 200 %. `-`/`+` and the `[`/`]` keys move it in 5 % steps, snapping an off-grid value
    such as 73 to 70 or 75.
  - The box takes a whole number. Values outside 25..200 are refused and marked in place.
  - Reset returns to 100 %.
  - The readout shows the section BPM × the speed (e.g. "60 BPM" for 120 at 50 %). It is empty without a tempo map.
- **Pitch kept** with Signalsmith Stretch (`signalsmith-stretch` `=0.1.3`, MIT, C++ vendored inside the crate).
  - At exactly 100 % the old playback path runs, bit for bit, including after returning to 100 % while playing.
  - Clicks are added after the stretch, so they stay on the music's beats and are never stretched themselves.
  - The count-in plays at the slowed tempo.
  - Positions, the waveform, sections and loops stay in song time.
- **Speed training** per loop, set in a popover opened by the "Train…" button in the Loops pane:
  - the popover has a Training on switch, Start %, End %, Step %, Repeats, a summary line and Close;
  - loop Play runs the passes (start speed, one step after the given repeats, then the end speed for ever; start may
    be above end);
  - the Loops status line shows "70 % · 2/4", or "80 % · 3" once at the end speed;
  - during a run the speed control is disabled (keys too) and shows the training speed;
  - Stop or Pause ends the run and returns to the track speed;
  - the loop's own Count-In rule applies, at the current pass speed.
- **Saving:** the speed (per track) and the training (per loop) are saved in `player.json`, now **schema 6**.
  - Files from the previous version open at 100 % with everything else unchanged.
  - A schema 7+ file is never overwritten.
  - Typing a BPM or Autodetect now keeps the speed.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| "-" ×3 → 85 %, 10 s of song take about 11.8 s | `sp01`; GUI `player_speed` | PASS |
| A 440 Hz tone stays at 440 Hz at 50 % and 150 % | `sp02`; `sp13` (25 %, 200 %) | PASS |
| 100 % → 70 % while playing: no pause, position keeps moving forward | `sp03`; `player_realtime`; `sp23` (back to 100 %) | PASS |
| 120 BPM at 50 % shows 60 BPM, clicks every 1 s on the beats | `sp04`; `player_realtime`; GUI `player_speed` | PASS |
| No tempo map: no BPM shown, speed still works | `sp05`; `ac13` | PASS |
| 20 and 250 refused, speed unchanged | `sp06`; `sp14`; `sp22`; frontend "refused typed values" | PASS |
| 80 % survives close and reopen | `sp07`; GUI `player_speed` | PASS |
| Training 60 → 80 %, step 10, ×2: passes and progress shown | `sp08`; `sp18` (slowing down); frontend progress test | PASS |
| Stop returns to the track speed, next Play restarts at 60 % | `sp09`; `sp19` (Pause) | PASS |
| Training settings survive close and reopen | `sp10`; frontend popover merge tests | PASS |
| A previous-version `player.json` loads at 100 %, unchanged | `sp11`; `sp15` (schema 7 untouched); `sp16` | PASS |
| Req. 13: count-in during training at the pass speed | `sp12` | PASS |

Tests are in `src/calliope-gui/tests/acceptance_player_backend.rs` (sp01–sp26, using ManualBackend; nothing waits in
real time), `tests/player_realtime.rs` (no allocation on the audio thread; clicks on the beats), the frontend tests
under `src/ui/` and the GUI test `player_speed` in `tests/gui_player_e2e.rs`.

## Task log

| Step | By | Result |
|---|---|---|
| plan | architect | 16 tasks; you chose the popover (Q1) and the recommended answers for Q2–Q10 |
| task 1: dependency, licences, notices | implementer | done |
| task 2: speed rules (`speed.rs`) | implementer | the local model's file was broken (missing struct, syntax error, no tests); discarded |
| task 3: stretcher wrapper | implementer | done; measured pitch, length and alignment |
| task 4: `player.json` schema 6 | implementer | done |
| task 8: schema numbers in tests | **local (gemma4)** + me | 15 of 23 edits, then its turn limit; I did the rest and one missed fixture |
| task 5: engine stretch mode | implementer | done |
| task 6: training in the engine | implementer | done; also fixed a jump into a loop's last ~120 ms skipping its end |
| task 7: manager and IPC | implementer | done |
| task 9: backend acceptance tests sp01–sp12 | implementer | done; no bugs found |
| task 10: TS types | implementer | done |
| task 11: speed text helpers | implementer | moved from the local model (new files with logic) |
| task 12: frontend state | implementer | done |
| task 13: Speed group, keys | implementer | done |
| task 14: "Train…" button and popover | implementer | done |
| task 15: GUI test, positions, docs | implementer + me | fixed a Speed-group clip and the popover placement; I reverted a wrongly moved Editor click position and a flaky click under the fading popover |
| task 16: README | **local (gemma4)** | correct on the first try (I made its "…" ASCII afterwards, my own prompt's mistake) |
| tester | tester | all criteria PASS; added sp13–sp21; no defects |
| reviewer | reviewer | CHANGES REQUIRED: 1 major, 5 minor, 1 nit |
| fix round 1 | implementer | all fixed |
| re-verify | tester, reviewer | APPROVED; added sp22–sp26, which found a loop-end bug |
| fix round 2 | implementer | loop-end bug fixed |
| re-verify | reviewer | APPROVED |

## Review findings and what happened to them

- **Major, fixed:** a speed step count sent from the webview had no bound. A huge value would have frozen the app
  while it counted. It is now capped (`MAX_STEPS`).
- **Fixed:**
  - closing the training popover with two edited fields could save one with its old value; it now sends one merged
    save;
  - returning to 100 % while playing kept the stretcher running; it now crossfades back to the exact 100 % path;
  - a speed change right after a training loop Play could slip through; the player now refuses it;
  - debug checks were added for two full buffers;
  - the crate is pinned to `=0.1.3`.
- **Fixed in round 2:** returning to 100 % within ~3 ms of a loop end made the loop play on to the track end (sp24,
  sp25).
- **Left as minor follow-ups (reviewer: fine to ship):**
  1. `SetSpeed(100)` arriving during the 3 ms fade into stretch mode stays in stretch mode at ratio 1.0. The audio is
     correct, but it costs a little CPU and is not bit-exact until the next jump, pause or play.
  2. If the playhead is already past a repeating loop's end at a non-100 % speed and you return to 100 %, the fade can
     jump back to the loop start instead of playing on. Reaching that state from the UI is unconfirmed.
  3. A jump into a repeating loop's last ~100 ms at a non-100 % speed skips up to ~120 ms at the start of the next
     pass (a short silence).

## Deviations from the plan

- The training button reads **"Train…"** (aria-label "Speed training settings"). "Training…" did not fit the line,
  so this is the plan's first fallback.
- The popover opens **to the left** of the button, not above it. At 1024x640 it was too tall to fit above. It now stays
  inside the window at 1280x800 and 1024x640, with the tree shown or folded.
- `Stretcher::align` returns the song frame to feed next, and the timeline keeps wraps and ends as separate events.
  These are internal choices recorded in the plan's orchestrator notes.
- The local model got only tasks 8 and 16. It broke task 2, and I moved task 11 to the implementer for the same
  reason.

## Try it

`cd src/calliope-gui && npm run dev:app`, or `npm run build:app` and run `target/release/calliope-gui`. Open a track
in the Player (Alt+5):

- use `-`/`+`, `[`/`]`, the % box or Reset in the Playback pane;
- for training, make a loop, press "Train…", set the speeds, switch Training on, and press the loop's Play.

**New build prerequisite:** `clang` (`sudo pacman -S clang`; already installed on archserver).

## Deployment

Laptop only: build with `npm run build:app` and install `target/release/calliope-gui` as before. Nothing changes on
archserver.

Install `clang` on the laptop if it builds there. This build writes `player.json` as schema 6. The previous build
opens those files read-only (it plays with defaults and never overwrites them), so don't mix the two builds on the
same repository.

## Manual checks (laptop, real audio)

- [ ] **M1, sound quality.** Use a real backing track with guitar and drums.
  - At 85, 70 and 50 % the pitch matches 100 %, and chords and drums are fine for practice (some softer attacks
    below 70 % are normal).
  - 120 % and 150 % pass the same check. 25 % and 200 % are usable, with artefacts expected.
  - A whole song at 70 % has no crackles or dropouts, also with the laptop busy.
- [ ] **M2, changing while playing.** `-`/`+`, `[`/`]`, a typed value and Reset change the tempo within a fraction of a
  second, with no gap, and the playhead keeps moving forward.
- [ ] **M3, metronome.** A track with sections and Click on, at 70 %:
  - the clicks stay on the song's beats and sound as crisp as at 100 %;
  - the readout shows the section BPM × 0.7;
  - Play's count-in is at the slower tempo.
- [ ] **M4, loops.** A loop at 70 % repeats with no gap or click at the wrap. Rew/FF, section Start/End and loop
  Prev/Next jump at once. Also try returning to 100 % just as the loop is about to wrap: it must still wrap.
- [ ] **M5, training.** Use a 2-bar loop.
  - Press "Train…", set 60 → 80, step 10, ×2, switch Training on, and close the popover with Escape (once more
    by clicking outside).
  - Loop Play plays two passes at 60, two at 70, then 80 for ever. Each change starts at the loop start with no gap.
  - The Loops pane shows "60 % · 1/2" and so on, and the speed box shows the training speed.
  - Stop returns to the track speed, and Play starts at 60 % again.
- [ ] **M6, saved.** Set 80 %, close the app and reopen the track: it is at 80 % and the training settings are there.
  An old track opens at 100 % with its loops and sections as before.
- [ ] **M7, CPU.** While playing at 70 %, `top` shows calliope-gui using only a few percent of one core more than at
  100 %.

Once you've verified it, tick `gui-player-speed.md` in the roadmap in `specs/overview.md`.
