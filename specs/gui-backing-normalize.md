# Normalize backing tracks

## Goal
<!-- One or two sentences: what problem does this solve, and for whom? -->

Give every backing track the same loudness, measured with ITU-R BS.1770 (the
loudness standard behind streaming platforms), so that switching tracks on the
RC-5 or in the Player doesn't need a volume change. New backing tracks are
normalized when they are saved; existing ones get a Normalize button instead of
having to be mixed again.

## Context
<!-- Background the team needs: existing system, users, why now. Link related specs. -->

- `gui-backing-track-editor.md`: the Editor mixes the stems of a stem track and
  Save renders the mix to a FLAC backing variant (`backings/<id>.flac`, listed
  in `track.json` with its mix). The lane shows "Save as: <variant>", a
  progress bar with Cancel, and a result line (amber when samples clipped).
- The Player, the export (`gui-export.md`) and the first-beat analysis all use
  the first backing variant.
- Backing tracks saved so far have whatever level the mix gave them; their
  loudness differs from track to track.

## Requirements
<!-- Numbered and testable. Say WHAT, not HOW. -->
1. **Loudness measure:** integrated loudness per ITU-R BS.1770 (K-weighting,
   gating as in BS.1770-4 / EBU R128), in LUFS, and the true peak in dBTP.
2. **Target:** the backing is scaled by one constant gain so its integrated
   loudness is the target (default -14 LUFS), unless that would push the true
   peak above the ceiling (default -1 dBTP); then the gain stops at the ceiling
   and the track ends quieter than the target. No limiting or compression: the
   sound is only made louder or quieter as a whole.
3. **On Save:** the Editor's Save writes the backing already normalized. Since
   the gain is applied before the samples are written, a mix that would have
   clipped no longer clips when the gain is lowered.
4. **Normalize button** next to Save in the Editor: it normalizes the existing
   backing variant shown in "Save as: ..." in place, without re-mixing the
   stems. Disabled when that variant has no file yet, while saving or
   normalizing, and while detecting. It shows progress and Cancel like Save;
   Cancel leaves the file unchanged.
5. **Result line** after Save and after Normalize, e.g. "Normalized to -14.0
   LUFS (gain +3.2 dB)" or "-15.6 LUFS (gain -1.0 dB, limited by the -1 dBTP
   peak ceiling)". A backing already within 0.1 LU of the target, or already at
   the ceiling, is left untouched and the line says so.
6. **Recorded:** the variant's entry in `track.json` records the measured
   loudness, true peak and the gain applied, so a later Normalize knows it is
   done; older builds keep reading the file.
7. **Everything else follows:** after Normalize, an open Player session of that
   track and the waveforms show the new file; the export uses it. The tempo map,
   the first beat, loops and the variant's mix are unchanged.

## Constraints
<!-- Language/framework, libraries allowed or forbidden, performance, platforms,
     where it runs, data it must not touch. Leave empty to let the architect choose. -->

- The measurement is checked against the EBU Tech 3341 / 3342 reference
  signals (or equivalent test tones with known loudness) within 0.1 LU.
- The file is replaced atomically: a failed or cancelled Normalize leaves the
  old file as it was.
- Tests use temporary repositories only, never the user's real repository,
  real devices or the real system dialogs.
- No `symphonia`.

## Out of scope
<!-- What NOT to build. This prevents gold-plating. -->

Limiting, compression or EQ. Normalizing stems, `audio`, `original` or plain
backing tracks (imported without stems). Batch normalization of the whole
library. A loudness meter while playing.

## Acceptance criteria
<!-- Concrete checks that prove it works. The tester turns each one into a test.
     Given <situation>, when <action>, then <observable result>. -->
- [ ] Given the EBU reference signals, then the measured integrated loudness
  and true peak are within 0.1 LU / 0.1 dB of their reference values
- [ ] Given a mix measuring -20 LUFS with a -10 dBTP peak, when it is saved,
  then the backing measures -14.0 LUFS (± 0.1) and the result line says gain
  +6.0 dB
- [ ] Given a mix whose -14 LUFS gain would push the peak above -1 dBTP, when it
  is saved, then the true peak is -1 dBTP (± 0.1), the loudness is below -14
  and the result line says it was limited by the ceiling
- [ ] Given a mix that clipped before this feature, when it is saved, then no
  sample clips and the clipped warning is not shown
- [ ] Given a backing saved before this feature, when Normalize is pressed,
  then the file measures -14.0 LUFS (± 0.1), its length and the stems are
  unchanged, and `track.json` records the loudness and gain
- [ ] Given a backing already normalized, when Normalize is pressed, then the
  file is not rewritten and the line says it is already at the target
- [ ] Given Normalize running, when Cancel is pressed, then the file is
  byte-identical to before
- [ ] Given the variant shown has no file yet, then Normalize is disabled
- [ ] Given the track open in the Player, when Normalize finishes, then the
  Player plays the normalized file

## Open questions
<!-- Things you haven't decided yet. The architect will ask about anything else it finds. -->

Answered by the owner (2026-10-28): the defaults below are accepted.
1. **Target:** -14 LUFS integrated (Spotify, YouTube). Alternatives: -16 LUFS
   (Apple Music), -23 LUFS (EBU R128 broadcast; quieter, more headroom).
2. **Peak ceiling:** -1 dBTP, gain stops there (no limiter), so very dynamic
   tracks end below the target.
3. **Which file Normalize acts on:** the variant shown in "Save as: ...". The
   other variants of the track stay as they are.
4. **Settings:** target and ceiling are fixed, not in Settings.
5. **Plain backing tracks** (imported, no stems) are left out; they have no
   Editor. A later feature could normalize them from the Library.
