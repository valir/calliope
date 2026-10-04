base: 76cf5fb
task 1 | Node/Vite/Svelte toolchain, Tauri pointed at dist/ | implementer | done (deviations: @types/node, vitest/config defineConfig, no baseUrl)
task 2 | shadcn-svelte components, theme tokens, bundled font | implementer | done (components.json hand-written: CLI init interactive; offline check made comment-aware to keep license banners)
task 3 | build.rs frontend check, window config, static CSP/asset tests | implementer | done
  - missing dist: "error: the frontend is not built (dist/index.html is missing). Build the app with: npm ci && npm run build:app   (or only the frontend: npm run build)"
  - stale: "warning: calliope-gui@0.1.0: the frontend in dist/ is stale (src/ui is newer than dist/index.html); run: npm run build"
  - note: rerun-if-changed=src/ui also covers the Vite cache in src/ui/node_modules/.vite (consider moving Vite cacheDir out of src/ui)
task 4 | Rust settings module | implementer | done
task 5 | IPC commands, window-state plugin, Tauri wiring | implementer | done (orchestrator tightened version test to exactly 4 digits)
task 6 | View registry and shortcut mapping | implementer (local failed: output limit) | done
task 7 | IPC wrappers, theme module, shared state | implementer | done
task 8 | Placeholder component and the four simple views | local | done
task 9 | Application shell (nav, footer, About, Track/Settings views, bootstrap) | implementer | done (screenshot on :1 OK)
  - note: App.test.ts Escape test waits a fixed ~50 ms for the bits-ui dialog to close (possible flake; prefer waitFor)
task 10 | GUI end-to-end tests on a display | implementer | done (DISPLAY=:1 npm run test:gui: 5 e2e + 1 smoke pass; 16 screenshots; Escape test now uses waitFor)
  - found by orchestrator in screenshots: Track view tabs laid out side by side with the panel, no active-tab highlight (task 9 defect) -> fixing next
fix | shadcn data-* variants never matched bits-ui 2 attributes (tabs/dialog/radio/separator): @custom-variant mappings in app.css + guard test | implementer | done (screenshots re-checked on :1)
task 11 | README and docs | implementer (local failed: output limit) | done
task 13 | Dev mode with hot reload (owner decision) | implementer | done (live check on :1: hot reload OK, 0 csp violations; release guard verified)
  - orchestrator: Cargo.toml tauri deps written in tauri-cli normal form so `tauri dev` no longer rewrites it
task 12 | Visual verification and fresh-clone build on :1 | implementer | done
  - (a) fresh clone (branch spec/gui-frontend-foundation): npm ci + npm run build:app OK; target/release/calliope-gui --version = "calliope-gui 26.10.0031"; launch on :1 with temp XDG dirs: ready view=library theme=dark version=26.10.0031 -> PASS (/tmp/claude-1000/-home-vali-src-calliope/4fa7c5ce-b6e9-431a-9d46-ad9fd99ccef5/scratchpad/t12-fresh.png)
  - (b) checklist, screenshots regenerated from HEAD (npm run build; gui_e2e: 5 passed), all in /home/vali/src/calliope/target/gui-shots/:
    1 dark bg, light text, amber accent on active nav: PASS (start-dark.png)
    2 nav order Library, Import, Track, Playlists, Player, Settings with Alt+1..6 hints: PASS (start-dark.png)
    3 titles + placeholders naming features; Track tabs (Stems/Assembly/BPM & sections/Tablature/MIDI cues, Stems active) and Settings cards (Appearance, MIDI, Audio, Edge-AI): PASS (view-*.png)
    4 footer "Ready", "Edge-AI: not configured", v26.10.0031 = --version: PASS (start-dark.png)
    5 light theme fully light (nav, content, footer, cards), persists after restart: PASS (settings-light.png, restart-light.png)
    6 at 1024x640 no overlap/clipping; Settings scrolls (allowed): PASS (min-library/import/track/playlists/player/settings.png)
    7 focus ring visible on nav/collapse button and theme radio (amber): PASS (/tmp/claude-1000/-home-vali-src-calliope/4fa7c5ce-b6e9-431a-9d46-ad9fd99ccef5/scratchpad/t12-tab3.png, settings-light.png)
    8 About dialog centred and readable with version: PASS (/tmp/claude-1000/-home-vali-src-calliope/4fa7c5ce-b6e9-431a-9d46-ad9fd99ccef5/scratchpad/t12-about.png)
    9 offline.png identical to start-dark.png (same 43749 bytes, same look): PASS
    10 Inter-like font, large readable text: PASS (start-dark.png)
  - (c) mouse clicks on nav entries via xdotool: stderr view=import, track, playlists, player, settings, library in order: PASS (/tmp/claude-1000/-home-vali-src-calliope/4fa7c5ce-b6e9-431a-9d46-ad9fd99ccef5/scratchpad/t12-click-{import,track,playlists,player,settings,library}.png); footer version click opens About: PASS; Tab shows focus ring: PASS
  - (d) window alone on empty workspace (1916x1120): layout fills sensibly, nav fixed width, content stretches, no awkward layout: PASS (/tmp/claude-1000/-home-vali-src-calliope/4fa7c5ce-b6e9-431a-9d46-ad9fd99ccef5/scratchpad/t12-maximised.png)
  - note: after programmatic view switch the h1 gets a visible amber focus ring (view-*.png, min-*.png); looks intentional (focus moved to heading for a11y) but is visually heavy; owner may judge.
  - note: checks done with a temporary workspace 9 (window moved there, then returned to workspace 3); app killed, no calliope-gui processes left.
tester | acceptance tests (tests/acceptance_gui_frontend.rs, src/ui/acceptance_frontend.test.ts) | all PASS | done
reviewer | 76cf5fb..HEAD | APPROVED (0 blockers, 0 majors, 6 minors) | done
