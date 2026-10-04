# Report: calliope-gui frontend foundation

**Status: DONE.** All acceptance criteria pass, the reviewer approved, and no fix rounds were needed.

- Branch: `spec/gui-frontend-foundation` (base `76cf5fb`), not pushed or merged
- Plan: `specs/gui-frontend-foundation.plan.md`, log: `specs/gui-frontend-foundation.log.md`

## Acceptance criteria

| Criterion | Tests / evidence | Result |
|---|---|---|
| Fresh clone, documented build command, binary opens the new UI | task 12 fresh clone (`npm ci && npm run build:app`, launched on `:1`); `req2_*` tests | PASS |
| Opens dark; nav shows Library, Import, Track, Playlists, Player, Settings | vitest shell tests; `gui_e2e::starts_dark_with_version`; `start-dark.png` | PASS |
| Each entry, by mouse and by shortcut, shows its placeholder naming the future feature | vitest; `gui_e2e::shortcuts_switch_views`; real mouse clicks on `:1` (task 12 and tester) | PASS |
| About / footer show the same version as `--version` | vitest; `req5_*`; e2e; About screenshot | PASS |
| Light theme switches the whole UI and survives a restart | vitest; `gui_e2e::theme_switch_persists`; `restart-light.png` | PASS |
| Size and position restored after a restart | `gui_e2e::window_state_and_min_size`: closed at (150,120) 1180x720, reopened at (150,120) 1180x720 | PASS |
| Nothing overlaps or is cut off at minimum size | `min-*.png` at 1024x640 for all 6 views (Settings scrolls) | PASS |
| Offline: looks the same | `dist_assets_are_local`, `req3_no_remote_references…`; `gui_e2e::offline_start` (`unshare -rn`) | PASS |
| `--help`/`--version` and CSP tests still pass | `acceptance_gui_skeleton`, `cli`, `frontend.rs` | PASS |
| `docs/architecture.md` covers the stack, commands, IPC pattern and GUI testing | `ac10_architecture_doc_is_updated` | PASS |

Suites:
- Headless (`npm test`, display unset):
  - svelte-check: clean
  - vitest: 37 passed
  - cargo: 17+13+10+3+10 passed, the 6 GUI tests skipped by default
  - clippy: clean
- GUI tests (`DISPLAY=:1 npm run test:gui`): 5 e2e tests and 1 smoke test passed.

## Owner decisions applied
- **Dev mode with hot reload.** The relaxed CSP lives only in `tauri.dev.conf.json`. It is the production CSP plus `'unsafe-inline'` in `style-src` and `ws://localhost:5173` in `connect-src`. Two guards keep it out of release builds: a check in `build.rs` and static tests. The production CSP is unchanged from gui-skeleton.
- **Nav order:** Library, Import, Track, Playlists, Player, Settings (Alt+1…6).

## Task log

| Task | Done by | Notes |
|---|---|---|
| 1. Node/Vite/Svelte toolchain | implementer | small toolchain fixes (`@types/node`, TS 6 without `baseUrl`) |
| 2. shadcn-svelte, theme, bundled font | implementer | `components.json` written by hand (the CLI `init` is interactive); I rejected a CSS plugin that stripped license notices; the offline check now ignores comments instead |
| 3. build.rs frontend check, window config, static tests | implementer | |
| 4. Rust settings module | implementer | |
| 5. IPC commands, window-state plugin | implementer | I tightened the version test to exactly 4 digits |
| 6. View list and shortcuts | implementer | the local model failed (output limit) |
| 7. IPC wrappers, theme, state | implementer | |
| 8. Placeholder and simple views | **local model** ✓ | exact content; I only added final newlines |
| 9. App shell | implementer | |
| 10. GUI e2e tests on `:1` | implementer | fixed 50 ms wait replaced by `waitFor` |
| fix | implementer | I found a broken Track view layout in the screenshots. Root cause: the shadcn components style themselves with `data-horizontal`/`data-active`/`data-open`…, but bits-ui 2 sets `data-orientation`/`data-state` instead. So tabs, dialog, radio and separator state styles never applied. Fixed with `@custom-variant` mappings plus a guard test |
| 11. README | implementer | the local model failed (output limit) |
| 13. Dev mode | implementer | ran before task 12; I put the Tauri deps in `Cargo.toml` in the form `tauri dev` writes, so dev runs no longer rewrite the file |
| 12. Visual check + fresh clone | implementer | every checklist item PASS |

The local model succeeded on 1 of 4 attempts (task 8). It handles exact-content files, but runs out of output on anything with free text.

## Reviewer: APPROVED (0 blockers, 0 majors)
Minor findings, suggested as follow-ups:
1. **Vite cache inside `src/ui`.** Vite and Vitest keep their cache in `src/ui/node_modules/.vite`, so after any frontend test run the next cargo command reruns the build script and rebuilds. Fix: set `cacheDir: '../../node_modules/.vite'` in `vite.config.ts`.
2. **Stale `dist/` only warns.** A `cargo build --release` after editing the UI ships the old UI, with only a warning. Consider failing for release builds. (`npm run build:app` always rebuilds.)
3. **Focus ring on the view heading.** It's a thick amber ring, shown after keyboard view switches only. It's correct per `docs/ui.md`, but visually heavy. A design call for you: keep it, or use a softer marker for the heading.
4. **`frontend_log`.** It's always on, and its stderr lines are the e2e tests' interface. Document that in the code, and consider limiting the `view=`/`theme=` lines to debug builds.
5. **One bad value resets all settings.** An invalid value in `settings.json` falls back to the defaults for everything, and the next save overwrites the file. Consider keeping a `.bak` copy before it grows more settings.
6. **No fsync before rename** when saving settings. Low impact today.

Nits:
- `set_theme` does file IO synchronously; heavier commands should be `async`.
- The e2e tests assume i3, xdotool and unshare.

Checked and fine: every attribute style the components actually use is covered by the mapping (`data-disabled`/`data-selected` are unused).

## Deviations from the plan
- Task 13 ran before task 12.
- `components.json` was written by hand.
- The offline/asset check ignores CSS comments, so license notices stay in the bundle.
- `Cargo.toml` Tauri deps are in the form `tauri dev` writes.
- The Track view fix landed after task 10.

## Notes from the run
- Agents hit permission denials several times: a combined shell command, `rm -rf` while cloning, and a shared cargo target dir. Each time they redid the step with scratch-only paths, without deleting anything.
- Task 12 briefly moved the app window to an empty i3 workspace on `:1` and switched back. The workspaces are as before.
- `target/release/calliope-gui` was stale: it still showed the gui-skeleton page until it was rebuilt with `npm run build:app`.

## How to try it
```sh
npm ci                  # once after cloning / after package-lock.json changes
npm run build:app       # -> target/release/calliope-gui
npm run app             # build + run (debug)
npm run dev:app         # hot reload (dev-only CSP)
npm test                # headless suite
DISPLAY=:1 npm run test:gui   # GUI e2e; screenshots in target/gui-shots/
```
Keys: Alt+1…6 switch views, Ctrl+B collapses the nav. Settings are saved in `~/.config/app.calliope.gui/` (`settings.json`, `.window-state.json`).

## Manual checks (your own desktop session)
- [ ] Fresh clone: `npm ci && npm run build:app` creates `target/release/calliope-gui`
- [ ] Opens dark, with amber on "Library"; the nav is Library, Import, Track, Playlists, Player, Settings
- [ ] Alt+1…6 and mouse clicks each show a view naming its future feature; Ctrl+B collapses and expands the nav
- [ ] Footer `v…` equals `--version`; clicking it opens About with the same version
- [ ] Settings → Theme → Light turns the whole UI light; it is still light after a restart (switch back afterwards)
- [ ] Resize and move, close normally, restart: same size and position (on i3 the window must be floating)
- [ ] Shrinking stops at 1024x640, with no overlapping or cut-off text in any view
- [ ] Maximised on 1920x1200, the layout looks right
- [ ] Network off: looks the same (font, icons)
- [ ] From 1-2 m in a dim room: the nav labels, placeholder text and focus ring are readable
- [ ] `--help`/`--version` print as before, without opening a window
- [ ] `npm run dev:app`: edit a text in `src/ui/views/LibraryView.svelte`, see it update live, then revert

Also: the ASCII sketch in `docs/ui.md` still shows the old nav order (Player before Playlists). Please update it.

No deployment steps: local build only.

Once you've verified it, tick **gui-frontend-foundation** in the roadmap of `specs/overview.md`.
