base: 76cf5fb
task 1 | Node/Vite/Svelte toolchain, Tauri pointed at dist/ | implementer | done (deviations: @types/node, vitest/config defineConfig, no baseUrl)
task 2 | shadcn-svelte components, theme tokens, bundled font | implementer | done (components.json hand-written: CLI init interactive; offline check made comment-aware to keep license banners)
task 3 | build.rs frontend check, window config, static CSP/asset tests | implementer | done
  - missing dist: "error: the frontend is not built (dist/index.html is missing). Build the app with: npm ci && npm run build:app   (or only the frontend: npm run build)"
  - stale: "warning: calliope-gui@0.1.0: the frontend in dist/ is stale (src/ui is newer than dist/index.html); run: npm run build"
  - note: rerun-if-changed=src/ui also covers the Vite cache in src/ui/node_modules/.vite (consider moving Vite cacheDir out of src/ui)
task 4 | Rust settings module | implementer | done
task 5 | IPC commands, window-state plugin, Tauri wiring | implementer | done (orchestrator tightened version test to exactly 4 digits)
