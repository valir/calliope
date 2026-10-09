# Report: Backing Track Assembly (Editor view)

**Status: DONE.** All 17 acceptance criteria and your 9 decisions pass. The reviewer approved after 1 fix round. No
test opened your real audio device; all of them used the silent, recording or test-only outputs.

- Branch: `spec/gui-backing-track-editor` (base `242562d`), not pushed or merged
- Plan: `specs/gui-backing-track-editor.plan.md`, log: `specs/gui-backing-track-editor.log.md`

## Acceptance criteria

Rust tests: `tests/acceptance_backing_editor.rs`. Frontend: `src/ui/acceptance_editor.test.ts`. GUI:
`tests/gui_editor_e2e.rs` (real app on `:1`, audio recorded to a WAV and checked for the expected tones).

| Criterion | Tests | Result |
|---|---|---|
| Track without stems: editor pane stays inactive | `ac1_*`; vitest; GUI `states_and_lanes` (Plain Backing, Missing Stem) | PASS |
| Track with stems: pane active, one lane per stem | `ac2_*` (4 and 6 lanes, `original.flac` hidden); GUI | PASS |
| Lane Play checks its Unmute and starts the mix playing | `ac3_*` (exact samples: only that stem heard); GUI solo with a WAV check | PASS |
| A checked stem enables Mix Play | `ac4_*`; vitest; GUI (Space and click do nothing with none checked) | PASS |
| Play plays the combined checked stems | `ac5_*` (sample-exact, dB gains, Off); GUI WAV: 440+220 Hz present, 660 Hz absent | PASS |
| Slider and time box follow playback | vitest; GUI | PASS |
| Play becomes Pause; Stop becomes enabled | vitest; `ac8_*` | PASS |
| Pause holds the position; Pause becomes Play | `ac9_ac10_*`; vitest; GUI | PASS |
| Typed time within range jumps there | vitest (`1:00.0`, end accepted, past end refused); `ac11_*` | PASS |
| Wheel up/down moves 0.1 s towards the start/end | vitest; `ac11_*_ac12_ac13_*`; GUI (3 wheel steps) | PASS |
| Adjusting while playing: silence, resume 0.3 s after the last change; the window resets | `ac14_ac15_*` (silent at +499 ms, resumes at +500 ms after two adjustments) | PASS |
| Save with a stem checked creates the backing file | `ac16_ac17_*` (sample-exact FLAC); GUI (file decoded and checked) | PASS |
| Lane volume contributes to the mix | `ac5_*`, `ac16_ac17_*` (-6 dB and +3.5 dB baked into the file) | PASS |

Suites (last run, after the fix round):
- Headless (`npm test`): vitest 369 passed; Rust 0 failed across the workspace; clippy clean (also with `e2e-hooks`).
- GUI (`DISPLAY=:1 npm run test:gui`): all 6 suites green, several runs in a row, no reruns needed after the fix.

## Your decisions applied
- **Time** shows as `3:07.4`. Typing also accepts `3:07:4`, `3:07`, `75` and `75.5`.
- **Volume** is in dB from -60 (shown as **Off**) to +12, in 0.5 dB steps, with a 0 dB tick. Clicking the dB value
  resets the lane to 0 dB.
  - Clipping is a hard clip, the same in playback and in the saved file.
  - A red **CLIP** badge stays lit for 1 s after the last clip, and Save reports how many samples were clipped.
- **Lane Play** is a temporary solo:
  - It checks that lane's Unmute, plays only that stem, and the button reads "Solo".
  - The other lanes are dimmed but keep their settings, and the Mix lane shows "Solo: Guitar".
  - The solo ends on Mix Play, Stop, the end of the track, the same lane's button, unchecking that stem, or a track or
    view change. Pause keeps it, and Save ignores it.
- **Starting mix:** a never-saved track opens with every stem unchecked at 0 dB, so Mix Play is disabled until you
  check one. A saved backing restores its mix, and unsaved settings are remembered while the app runs.
- **Playback stops** when you select another track and when you leave the Editor.
- **Backing variants, ready for several:**
  - `track.json` gets a `backings` list (`id`, `name`, `file`, timestamps, rate, bits, `mix`). Files are stored as
    `backings/<id>.flac`.
  - This feature saves one variant, "Backing". Saving again replaces it, and the old file goes to
    `trash/<stamp>-<id>-backing/`.
  - A file of yours already at that name is never overwritten: Calliope writes `backing-2` instead.
  - `audio` stays `null` for stem tracks. The Library shows a read-only "Backing tracks" row.
- **`original.flac`** is hidden in the Editor.
- **No symphonia:** recorded as a permanent decision in `docs/architecture.md` and `docs/licences.md`.
- **FLAC output**, at the stems' sample rate, stereo, 16-bit (24-bit if any stem is).

## Design summary
- **All audio runs in Rust:** `claxon` decodes, `flacenc` encodes, and `cpal` plays through ALSA, which reaches
  PipeWire. The webview plays nothing, so the production CSP is unchanged.
- **Playback:**
  - Opening a track decodes its stems into memory as 16-bit; the cap is 1.5 GiB, with a clear message.
  - The mixing callback takes no locks and allocates nothing. Gains and position are atomics.
  - Save re-reads the stems at full precision and uses the same mix function, so the file matches what you heard.
- **Save** checks that the track's stem list is unchanged on disk instead of using the usual revision check, so a
  Library edit made meanwhile doesn't block it.
  - It writes to a `.calliope-backing-*.part` file, moves any old file to the trash, links the new one in without
    overwriting anything, and writes `track.json` atomically.
  - Any failure rolls back.
- **Interface:**
  - The Library's tree column is now a shared `TrackBrowser`, so the selected track is the same in both views.
  - The mixer is in the Editor's "Stems" tab, and the "Assembly" placeholder tab is gone.
  - The new commands bring the app's permission list to 40 commands. The interface still never sends a file path.

## Task log

| Task | Done by |
|---|---|
| 1 fixtures · 2 audio crates + licences | implementer (the local model failed both) |
| 3 stem decoding · 4 mixer · 5 transport · 6 audio outputs | implementer |
| 7 editor engine · 8 `backings` + `save_backing` · 9 render + save job · 10 IPC | implementer |
| 11 time format · 12 gain format | **local model** ✓ (exact content and test table given) |
| 13 shared tree + layout · 14 slider/checkbox · 15 frontend state | implementer |
| 16 lanes + Mix lane · 17 time field · 18 GUI e2e · 19 visual review + docs | implementer |
| fix round 1 | implementer |

- **Local model:** with Ollama now at 64k context it no longer runs out of output, but tasks 1 and 2 were still wrong.
  - Task 1 used the wrong folder layout and never ran its script.
  - Task 2 deleted `[workspace]` from `Cargo.toml` and duplicated sections.
  - It got the two pure functions with exact test tables (tasks 11 and 12) right.
- **Bug found by the GUI tests (task 18):** the position slider rounded the playing position to its 100 ms step and
  reported that as a user move. So playback seeked on every update and stuttered forward in 100 ms jumps. It's fixed,
  and a GUI test now asserts that no seek happens while playing.

## Review
- **First review: CHANGES REQUIRED.**
  - **The major:** the sound device was opened and closed while the Editor's lock was held. A stalled
    PipeWire/ALSA could freeze the Editor and block the app's exit.
  - Eight minor findings came with it.
  - **The tester:** all criteria passed, plus three defects:
    - a stem replaced on disk at another sample rate was saved sped up;
    - the Editor GUI test could run without the silent-audio feature;
    - the time-message wording.
- **Fix round 1:**
  - **Device and shutdown:** the device is opened and closed outside the lock and off the async runtime. Shutdown is
    bounded, still cancels a running save, and is tested with a fake device that hangs.
  - **Failed opens and crashed saves:** a failed device open changes no state, and a crashing save thread frees the
    slot and removes its part file.
  - **Fast track switching:** the newest open owns the event channel, and A→B→A retries instead of hanging on
    "loading".
  - **Tests and real-time code:**
    - the Editor GUI test refuses to run without `e2e-hooks`;
    - the device callback never allocates;
    - the position slider no longer jitters after a seek;
    - a stem whose rate changed fails the save with a message naming it;
    - the time message is "Enter a time between 0:00.0 and \<duration\>".
- **Re-review: APPROVED.** The re-test passed with 2 new probes (fast switching, a failed device open).

**Remaining optional minors (not fixed):**
- If the session is replaced while Play is still opening the device, that device is closed while the lock is held.
  This is a small remnant of the major; the fix is a few lines in `play_with`/`lane_play_with` in `editor.rs`.
- After a "Check a stem first" refusal, the opened output stays open until the next Stop. Harmless.
- If a load thread panics, `watch_editor` stops taking over the event channel. Very unlikely.
- Stop can itself wait on a hung device, though the rest of the Editor keeps working.
- The Editor GUI tests click at fixed coordinates in a 1280x800 window. The layout no longer shifts, but any future
  layout change needs the coordinates updated.
- Decoding follows a stem file that is a symlink, as the other repository reads do. Only FLAC audio is parsed.

## Deviations from the plan
- Lane Play on the stem already soloed ends the solo without starting playback. Mix Play at the end of the track
  restarts from 0.
- Output length is the longest stem of the track, not only of the checked ones, so the file matches playback.
- In the plan's GUI step, Mix Play during a solo pauses (it's the Pause button while playing). The test pauses, then
  plays, and the solo ends.
- Re-attaching the Editor after a webview reload was wired in task 16, since no task owned it.
- **Screenshots:**
  - The 1920x1200 screenshots weren't taken: when the window was resized to that size, the webview only painted about
    1598x958.
  - At 1024x640 only about 1.5 lanes are visible above the Mix lane. This is recorded in `docs/ui.md` as a known
    limit; Ctrl+B gives more width, and a narrower tree in the Editor would help.

## Follow-ups
- The optional minors above, especially the device-close-under-lock remnant.
- Locate GUI elements by `data-testid` bounds instead of coordinates, in the Editor and the Library e2e tests.
- Several named backing variants (UI to name and choose them). The data shape is already in place.
- The Player can use `backings[0]` as the default backing, and ffmpeg plus claxon for other formats (no symphonia).

## How to try it
```sh
npm ci && npm run build:app && target/release/calliope-gui    # needs alsa-lib
cp -r tests/fixtures/library-editor ~/calliope-editor-demo    # then Settings > Track repository > Choose folder
DISPLAY=:1 npm run test:gui    # GUI e2e (silent audio); screenshots in target/gui-shots/editor-*.png
```
Keys in the Editor: Alt+3 opens it, Space plays/pauses, Ctrl+S saves, Ctrl+F searches the tree. Wheel over the time
field to move 0.1 s per step.

## Manual checks (your laptop, real audio)
- [ ] `npm run build:app`, run `target/release/calliope-gui`, and pick a real imported stem track in the Editor:
  - 6 lanes appear, all unchecked at 0 dB;
  - Mix Play is disabled;
  - "Loading stems" finishes within a few seconds.
- [ ] Check every stem except Guitar and press Mix Play. The mix without guitar plays on the default output, and the
  time (`0:12.3`) and slider move smoothly.
- [ ] While playing, check/uncheck stems and drag Vocals to -12 dB, then +6 dB. The change is heard at once, and the
  value text matches.
- [ ] Press Play on the Guitar lane:
  - only the guitar is heard;
  - its checkbox is checked, and the button reads "Solo";
  - the other lanes keep their checkboxes.

  Then press Pause and Play on the Mix lane: the full mix returns, guitar included. Uncheck Guitar.
- [ ] Push two stems to +12 dB: CLIP lights and you hear the distortion. Bring them back: the badge goes out after
  about 1 s.
- [ ] Wheel over the time field while playing: the sound stops and resumes about 0.3 s after the last step. Wheel up
  goes back and wheel down goes forward, 0.1 s per step.
- [ ] Type `1:00.0` + Enter: it jumps to one minute. Pause/Play resumes in place. Stop returns to `0:00.0`.
- [ ] Switch to the Library while playing: playback stops. Select another track in the Editor while playing: it stops.
- [ ] Save:
  - progress shows, then "Saved.";
  - `<track>/backings/backing.flac` played in another player matches what you heard;
  - the Library shows "Backing tracks: Backing (backings/backing.flac)";
  - after restarting the app, the mixer shows the saved settings.
- [ ] Save again with a different mix: the old file is in `<repository>/trash/…-backing/`.
- [ ] A 10–15 minute track: loading time and memory use (system monitor) are acceptable.
- [ ] Set a headset as the system default output and play: the sound goes there.
- [ ] Unplug or suspend the output while playing: an audio error is shown and the app stays responsive.
- [ ] Readable from 1–2 m in a dim room, and a keyboard-only run works (Tab through the lanes, Space, arrows on
  the sliders).

No deployment steps: local build only, with the new prerequisite `alsa-lib`. Nothing changes on archserver.

Once you've verified it, tick **gui-backing-track-editor** in the roadmap of `specs/overview.md`.
