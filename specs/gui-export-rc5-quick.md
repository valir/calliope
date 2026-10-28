# Quicker RC-5 exports

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

When several tracks are exported to the BOSS RC-5 one after the other, don't
ask for the RC-5 folder every time: once it has been picked in the Export view,
the next RC-5 exports go straight to it while the RC-5 is still mounted there.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `gui-export.md`: "to BOSS RC-5" opens the system folder dialog at the
  remembered RC-5 folder (`rc5_dir` in the settings); the chosen folder must
  hold a `ROLAND` folder; an existing memory file is only overwritten after a
  confirmation that names it. `gui-export-info.md` added the memory box,
  filled from the track's recorded memory.
- The owner mounts the RC-5 before starting the app, then exports a batch of
  tracks in the Export view: select a track, set the memory, "to BOSS RC-5",
  confirm the dialog, and again for the next track. The dialog is the same
  every time.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. **First RC-5 export in the Export view:** unchanged: the folder dialog
   opens at the remembered folder.
2. **Next RC-5 exports in the same visit of the Export view:** after an RC-5
   export has succeeded, the following "to BOSS RC-5" clicks skip the folder
   dialog and export to that same folder at once, as long as it still exists
   and still holds a `ROLAND` folder.
3. **The RC-5 is gone:** if that folder no longer exists or has no `ROLAND`
   folder (e.g. the RC-5 was unplugged), the folder dialog opens again as in
   requirement 1.
4. **Overwrite:** the overwrite confirmation is kept exactly as it is: an
   existing memory file is never replaced without it, and cancelling it writes
   nothing.
5. **Leaving the Export view** (another view, or closing the app) ends the
   visit: the next RC-5 export opens the folder dialog again.
6. **Which folder:** the RC-5 row shows the folder the next export will use
   without asking (e.g. "To: /run/media/me/BOSS_RC-5"), with a "Change..."
   button that opens the folder dialog for the next export.
7. MP3 and OGG exports are unchanged (they always open the folder dialog).
8. The Export pane still fits at 1280x800 and 1024x640 without overflow.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- Tests use temporary directories only: never a real RC-5, a real drive or the
  user's real repository, and never the real system dialogs (the scripted
  picker).

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

Skipping the dialog across app restarts or after leaving the Export view.
Detecting the RC-5 automatically (watching mounts). Skipping the overwrite
confirmation. MP3/OGG.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given the Export view just opened, when "to BOSS RC-5" is clicked, then
  the folder dialog opens at the remembered folder
- [ ] Given an RC-5 export to `B` succeeded in this visit, when another track
  is selected and "to BOSS RC-5" is clicked, then no dialog opens and the file
  is written under `B/ROLAND/WAVE/`
- [ ] Given that, when the memory file already exists, then the overwrite
  confirmation names it, and cancelling it writes nothing
- [ ] Given an RC-5 export to `B` succeeded, when `B/ROLAND` is removed and
  "to BOSS RC-5" is clicked, then the folder dialog opens
- [ ] Given an RC-5 export succeeded, when the user goes to the Player and
  back to Export, then the next RC-5 export opens the folder dialog
- [ ] Given an RC-5 export to `B` succeeded, then the RC-5 row shows `B`, and
  "Change..." opens the folder dialog; the folder picked there is used
- [ ] Given an RC-5 export that failed or was cancelled, then the next one
  opens the folder dialog
- [ ] Given an MP3 export succeeded, when MP3 is clicked again, then the
  dialog opens
- [ ] Given the Export view at 1280x800 and 1024x640, then nothing overflows

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

Proposed defaults, to confirm or change:
1. **When the skip starts:** after the first RC-5 export of the visit that
   *succeeded* (not merely a confirmed dialog; a failed or cancelled export
   asks again).
2. **"Change..."** button and the "To: <folder>" line on the RC-5 row, so the
   folder can be changed without leaving the view.
3. **RC-5 only:** MP3/OGG keep opening the dialog every time.
4. **Ending the visit:** any other view, or closing the app. Selecting another
   track, band or album inside the Export view keeps it.
