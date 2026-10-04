# calliope-gui frontend foundation

## Goal
Replace the static "hello" page with a real frontend foundation: a build toolchain, a
consistent look, and the navigation shell with every main view as a placeholder. Every
later feature then adds its screens into this frame, instead of each feature inventing
its own structure and style.

## Context
- `gui-skeleton` produced a Tauri 2 window showing a static page from `src/ui/`, with no
  node toolchain (see the decision log in `docs/architecture.md`, which already named
  "tablature rendering" as the trigger to revisit that).
- Upcoming features need real UI: the track repository, stem extraction progress, backing
  track assembly, tablature display (planned with alphaTab, an npm package), MIDI cue
  editing, playback, and playlists.
- The look and behaviour are described in `docs/ui.md`. Follow it; where it's silent, choose
  simple, consistent defaults and record them there under "Decided by the team".
- The agents can check the GUI visually: display `:1` with `gui-shot` screenshots.

## Requirements
1. The frontend is written in **TypeScript** with a component framework and a build step.
   Its source stays under `src/` (owner rule: all source in `src/`).
2. Building and running the app are each **one documented command**, in the README. Using
   `cargo build` alone either still works, or fails with a clear message telling the
   developer which command to use instead.
3. A **component library and theme** provide the consistent look: colours, typography,
   spacing, buttons, inputs, dialogs and tabs. The **dark theme is the default**, and a light
   theme exists and can be switched in Settings. Fonts and assets are bundled in the app
   (no CDN): the app must work offline.
4. A **navigation shell** with these views, each reachable from the main navigation and
   each showing a placeholder that says which feature will fill it:
   - **Library**: the backing-track repository (`gui-tracks-repository`)
   - **Import**: import a music track for stem extraction or an existing backing track
     (`gui-stem-extracting`, `gui-existing-track-import`)
   - **Track**: the details of one track: stems, assembly, BPM/sections, tablature, MIDI cues
     (`gui-backing-track-assembly`, `gui-tablatures`, `gui-manipulate-backing-track`)
   - **Player**: playback with the tablature view (`gui-play-backing-track`)
   - **Playlists**
   - **Settings**: MIDI interface, audio output, edge-AI server, theme
5. The **first Rust ↔ frontend connection**: a Tauri command returns the app version
   (`YY.MM.BBBB`, the same value as `--version`), and the frontend shows it in the About
   panel / footer. This establishes the pattern later features follow for IPC.
6. Keyboard navigation: every view is reachable by keyboard, the focus is visible, and
   there are shortcuts for switching views (the defaults are in `docs/ui.md`).
7. The window remembers its size and position between runs, and has a minimum size at which
   the layout still works (`docs/ui.md`).
8. The CSP stays strict, as decided in gui-skeleton. If the chosen toolchain needs a
   relaxation, stop and ask the owner. Don't loosen it silently.

## Constraints
- **Svelte 5 + TypeScript + Vite**, with **shadcn-svelte** (Tailwind-based) as the component
  library (owner decision). Reasons: little code that is easy to read, and components are
  copied into the repo, so they're easy to change. Tauri and alphaTab both work with it.
- Logic that matters for timing (audio, MIDI, sync) stays in Rust, as the overview says.
  The frontend displays and sends commands; it does not keep time.
- Linux (Arch, webkit2gtk-4.1) is the platform to verify now. Don't use web features that
  only work in Chromium.
- Prefer a small number of well-maintained dependencies.

## Out of scope
- Real content in any view (track lists, import flows, the player, the tablature
  rendering): those are the features named in requirement 4.
- Persisting the settings (MIDI device, audio output, server). The Settings view shows the
  sections as placeholders, except the theme switch.
- Installers and packaging for Windows/macOS.
- Localisation (the UI is English).

## Acceptance criteria
- [ ] Given a fresh clone with the documented prerequisites, when the documented build
      command runs, then it produces a `calliope-gui` binary that opens the new UI.
- [ ] Given the app is started, when it opens, then the dark theme is active and the
      navigation shows Library, Import, Track, Player, Playlists and Settings.
- [ ] Given the app is open, when each navigation entry is chosen (by mouse and by its
      keyboard shortcut), then that view's placeholder is shown, naming its future feature.
- [ ] Given the app is open, when About / the footer is viewed, then it shows the same
      version string that `calliope-gui --version` prints.
- [ ] Given the Settings view, when the theme is switched to light, then the whole UI changes
      to the light theme, and it stays light after a restart.
- [ ] Given the window was resized and moved, when the app is restarted, then it reopens
      with the same size and position.
- [ ] Given the window is at its minimum size, when each view is shown, then nothing
      overlaps or is cut off (checked by screenshots).
- [ ] Given the network is unavailable, when the app starts, then it looks the same (no
      remote fonts or assets).
- [ ] Given the existing test suite, when it runs, then `--help`/`--version` and the CSP
      test still pass.
- [ ] `docs/architecture.md` describes the frontend stack, the build commands, the IPC
      pattern, and the corrected GUI testing approach (display `:1` + `gui-shot` is available
      to agents; the old "no display server" note is outdated).

## Decided
- Stack: Svelte (see Constraints).
- The **Roland FC-300 never controls Calliope**: its footswitches control only the gear.
  The direction is the opposite: Calliope will later *send* MIDI to the FC-300 (and the
  gear). So Calliope needs no MIDI input handling, and during playback it's driven by the
  keyboard (see `docs/ui.md`).

## Open questions
