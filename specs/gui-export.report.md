# Report: Export to the BOSS RC-5, MP3 or OGG

**Status: DONE.** All acceptance criteria and requirements 1-12 pass. The tester found one low-severity defect, and
the reviewer asked for one fix. One fix round fixed both, and the reviewer approved. The full headless suite (755
frontend tests and all Rust tests, clippy clean) and the full GUI suite (9 binaries) pass on `9f15ad0`. Branch
`spec/gui-export`, base `852b4d4`, not pushed or merged.

## What was built

- **Export view** (Alt+6; Settings moves to Alt+7):
  - The Library tree is on the left: read-only, in the same order, with search and the keyboard keys.
  - On the right are the selected track's band, album and title, the backing track's file, a count-in line,
    and the "Export..." group: **to BOSS RC-5** (with a Memory box), **MP3 file** and **OGG file**.
- **When the buttons are enabled:**
  - for a stem track with a saved mix, or for a plain (imported) backing track;
  - only when that track has a tempo map.
  - Otherwise the pane gives the reason: "No mix saved yet: save one in the Editor", "Set a tempo in the Player
    first: the export starts with a count-in", or "Select a track to export it" for a band or album row.
- **Count-in** (your requirement 12 and answers):
  - Every file is 4 beats of silence followed by the original, unchanged from its 0:00. The beats are at the
    first section's BPM; the Player speed is ignored.
  - Six Player clicks sit at the first beat + 0, 1, 2, 2.5, 3 and 3.5 beats, so the count ends exactly on the
    song's first beat. Anything before the first beat (silence, noise, a pickup) stays where it was.
  - The first beat is read from the tempo map at export time, so your planned Editor alignment will apply.
  - The length is always the original plus 4 beats, computed in exact frames.
- **to BOSS RC-5:**
  - You type the memory (1-99) and pick the RC-5's root folder.
  - The file is written as `<root>/ROLAND/WAVE/0NN_1/0NN_1.WAV` (`WAVE` and `0NN_1` are created when missing).
    It is 32-bit float, 44.1 kHz, stereo, and holds only the `fmt ` and `data` chunks. Other sample rates are
    resampled, and mono becomes stereo.
  - A folder without `ROLAND` is refused: "No ROLAND folder in <folder>: is the RC-5 mounted?".
  - Other files in the memory folder are left alone.
- **MP3 / OGG:**
  - Files are named `<Band> - <Title>.mp3`/`.ogg`, with invalid characters replaced.
  - MP3 is VBR quality 2 and Ogg Vorbis quality 6, at the track's own rate.
  - Tags: title, band as artist, album, track number (when the track is in an album) and year.
- **Safety:**
  - An existing file is replaced only after a confirmation that names it.
  - The export writes a hidden part file next to the target, syncs it to disk and renames it into place. Cancel
    and failures remove it, along with any folders the export created.
  - A destination inside the repository is refused, and the repository is never changed.
  - Missing ffmpeg/ffprobe or a missing encoder is reported before the folder dialog opens.
- **While it runs:**
  - The pane shows progress with Cancel; success shows the full path (RC-5: "Eject the RC-5 before unplugging
    it.").
  - Every step is logged (`calliope: export ...`).
- **Remembered** in `settings.json`:
  - the RC-5 root and the MP3/OGG folder, separately;
  - the last memory. The box then suggests that number plus one; 99 stays 99.
  - A remembered RC-5 folder that is unplugged is kept, and the dialog then opens in your home folder.

## Acceptance criteria

| Criterion | Test | Result |
|---|---|---|
| Stem track with a saved mix: buttons enabled | `ac_stem_track_with_mix_is_exportable`; `ExportView.test.ts`; GUI `export_view_buttons` | PASS |
| Stem track without a mix: disabled, "no mix saved yet" | `ac_stem_track_without_mix_is_not_exportable`; `ExportView.test.ts`; GUI `export_view_buttons` | PASS |
| Band or album selected: disabled | `ExportView.test.ts` (with a track selected outside the tree afterwards); GUI `export_view_buttons` | PASS |
| Plain backing track: enabled | `ac_plain_backing_is_exportable`; GUI `export_view_buttons` | PASS |
| No tempo map: disabled (your answer 2) | `ac_track_without_tempo_is_not_exportable`; `ExportView.test.ts`; GUI `export_view_buttons` | PASS |
| Memory 7 → `ROLAND/WAVE/007_1/007_1.WAV`, float32, stereo, 44.1 kHz, `fmt `+`data` only, original + 4 beats | `ac_rc5_memory_7`; GUI `export_rc5_flow` | PASS |
| No `ROLAND` folder: refused, nothing written | `ac_rc5_without_roland`; `ac_rc5_refusals_write_nothing`; GUI `export_rc5_refusals` | PASS |
| Existing `007_1.WAV`: the confirmation names it, Yes replaces it | `ac_rc5_overwrite_confirmed`; GUI `export_rc5_flow` | PASS |
| Mono → stereo with the same signal on both channels | `ac_rc5_mono_becomes_stereo`; GUI `export_rc5_flow` | PASS |
| Memory 0, 100, "abc": refused, nothing written | `ac_rc5_bad_memory_numbers`; GUI `export_rc5_refusals` | PASS |
| 48 kHz → 44.1 kHz, same duration + 4 beats | `ac_rc5_resamples_48k` | PASS |
| MP3 `<Band> - <Title>.mp3`, tagged, duration + 4 beats | `ac_mp3_export`; `ac_encoder_settings_and_tags_reach_ffmpeg`; GUI `export_mp3_and_ogg` | PASS |
| OGG: Vorbis, duration + 4 beats | `ac_ogg_export`; GUI `export_mp3_and_ogg` | PASS |
| Overwrite declined: file unchanged | `ac_overwrite_declined`; GUI `export_rc5_flow` | PASS |
| Cancel: no file left | `ac_cancel_leaves_nothing`; `ac_cancelled_mp3_leaves_nothing_...`; GUI `export_cancel_and_failure` | PASS |
| Unwritable destination: fails with its reason | `ac_unwritable_destination`; `ac_failing_encoder_...`; GUI `export_cancel_and_failure` | PASS |
| Track folder and `track.json` unchanged | every `ac_*` test ends by checking the repository; GUI tests too | PASS |
| Req. 12: count-in on the beat grid, ending on the first beat | `ac_count_in_before_the_first_beat`; `ac_count_in_odd_beat_length_...`; `ac_count_in_ignores_speed_and_bar_length`; `ac_count_in_uses_the_first_section_only_...` | PASS |
| Req. 10: remembered folders and memory after a restart | `ac_remembered_after_restart`; GUI `export_rc5_flow` | PASS |
| Req. 11: Alt+6 Export, Alt+7 Settings | `views.test.ts`; `acceptance_frontend.test.ts`; GUI `gui_e2e` | PASS |

The tester also covered:
- memory boundaries (1, 99, " 7 ", "007");
- invalid file-name characters with the real text kept in the tags;
- a destination inside the repository;
- missing tools or encoders (fake ffmpeg);
- a long intro kept as is;
- a first beat moved after the folder was chosen.

The headless tests are in `src/calliope-gui/tests/acceptance_export.rs` (31), the GUI tests in
`tests/gui_export_e2e.rs` (5), and the view tests in `src/ui/views/ExportView.test.ts` (15).

## Task log

| Step | By | Result |
|---|---|---|
| plan | architect | 17 tasks; your answers: length = original + count-in, no tempo = disabled, strictly on the beat grid |
| tasks 1-5: settings, fixtures, WAV writer, piped stdin, rules | implementer | done; `file_name` also takes the track id as a fallback |
| tasks 6-9: count-in pipeline, job, commands, acceptance tests | implementer | done; the memory is saved just before "done" is reported |
| tasks 10-15: navigation, Alt+7, TS types, view and pane | implementer | done; an old placeholder test now checks the real view |
| tasks 16-17: GUI tests and screenshots, docs | implementer | done; fixed a 10 px overflow at 1024x640 |
| GUI suite | me | all 9 binaries pass |
| tester | tester | all PASS; 14 tests added; 1 low defect |
| reviewer | reviewer | CHANGES REQUIRED: 1 major, 3 minor, 3 nits |
| fix round 1 | implementer + me | fixed (below); both suites pass |
| re-verify | tester, reviewer | defect fixed; APPROVED |

The local model was not used. My notes show it breaking new files with logic and edits inside long files, which
covers every routine task in this plan.

## Findings

Fixed in round 1 (`9f15ad0`):
1. **Stale band focus (major).** Say a band row was focused last and you then selected a track another way (Show
   in Library, the Player). The Export pane kept "Select a track to export it". Selecting a track now clears it.
2. **Missing guard (tester).** The export function now refuses a track that isn't exportable itself, instead of
   relying only on the disabled buttons.
3. **905 s cap (minor).** A track longer than about 15 minutes would have been cut without a word. The whole
   track is now always exported.
4. **Repository switched mid-export (minor).** The tempo is now read from the track folder recorded when the
   export was prepared.

Left as is:
- **Overwrite race (accepted, recorded in `docs/architecture.md`).** If another program creates the target in
  the moment between the existence check and the rename, it is replaced.
- **Nits:**
  - The token is used up even when the start is refused; the pane says to press again.
  - A source that decodes to nothing gives a file holding only the count-in.
  - The created RC-5 folders are not synced into their parents.

## Deviations from the plan

- The fixture has 10 tracks, not "nine" (the plan's table had 10).
- `export_rules::file_name` takes the track id as a fourth argument, for names that sanitize to nothing.
- The memory is saved just before the "done" event, so the next suggestion is right when the pane refreshes.
- The 905 s decoder cap from the plan is removed (finding 3).
- The GUI tests click the buttons at fixed coordinates and read results from the log lines, because the test
  harness can't read the page.
- An old navigation test that expected a placeholder for every view now checks the real Export view.

## Suggested follow-ups

- If the RC-5 refuses the WAV (manual check 1), add a `fact` chunk.
- Your planned Editor feature for placing the first beat precisely. The export already follows it.
- At 1024 px the count-in line takes two lines. This is noted in `docs/ui.md`.

## Try it

`cd src/calliope-gui && npm run dev:app`, or `npm run build:app` and run `target/release/calliope-gui`. Press
Alt+6, select a track with a saved mix and a tempo, then press "to BOSS RC-5", "MP3 file" or "OGG file".

## Deployment

The laptop app only: `npm run build:app` and install as before. Nothing changes on archserver.
- `settings.json` gains three optional fields (`rc5_dir`, `export_dir`, `rc5_memory`), which older builds
  ignore.
- ffmpeg and ffprobe are still required. Arch's `ffmpeg` package includes libmp3lame and libvorbis.

## Manual checks (laptop, real RC-5)

- [ ] **M1, RC-5 import.** Mount the RC-5 by USB, select a track with a saved mix and a tempo, keep memory 7 (or
  an empty one), press "to BOSS RC-5" and choose the drive's root.
  - "Written to .../ROLAND/WAVE/007_1/007_1.WAV" appears.
  - Eject, unplug, select memory 7 on the unit and play: after 4 beats the track plays, and the six ticks lead
    exactly into the first beat.
  - If the unit refuses the file, tell me (a `fact` chunk may be needed).
- [ ] **M2, not the root.** Choose the `ROLAND` folder itself: "No ROLAND folder in ...: is the RC-5 mounted?",
  and nothing changes on the drive.
- [ ] **M3, overwrite.** Export another track to the same memory: the confirmation names `.../007_1/007_1.WAV`.
  No keeps the old loop and Yes replaces it; check on the unit.
- [ ] **M4, remembered.** Restart Calliope: the memory box shows the last number plus one, and the dialog opens on
  the RC-5 root. With the RC-5 unplugged it opens in your home folder.
- [ ] **M5, no tempo.** A track whose tempo was never set has the buttons disabled with "Set a tempo in the Player
  first...". Set a BPM in the Player and come back: they are enabled.
- [ ] **M6, MP3/OGG.** Export as MP3 and OGG to `~/Music/exports`.
  - The files are named "<Band> - <Title>".
  - Your player shows the right title, band, album, track number and year.
  - The length is the track's plus 4 beats.
- [ ] **M7, cancel and unplug.** Cancel an RC-5 export of a long track: no new file in the memory folder. Start
  another and pull the cable mid-way: it fails with a reason, and after re-plugging no `.part` file is left. A
  crash or power loss may leave a hidden `.calliope-...part`; delete it by hand.
- [ ] **M8, listen.** Play the MP3 next to the Player: the backing sounds the same, and the count-in is the
  Player's click.

You've already ticked `gui-export.md` in the roadmap in `specs/overview.md`. Untick it until the manual checks
pass, if you want the roadmap to reflect verified features only.
