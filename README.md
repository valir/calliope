# Calliope

Calliope is a music player assistant for guitar players: a backing-track library, stem
extraction on an edge-AI server, tablatures shown in sync with playback, and MIDI control of
the guitar gear. The big picture is in [`specs/overview.md`](specs/overview.md).

## Repository layout

A Cargo workspace with one folder per crate under `src/`:

| Folder | What it is | README |
|---|---|---|
| `src/calliope-gui/` | the desktop app (Rust + Tauri 2, Svelte frontend); runs on the laptop | [README](src/calliope-gui/README.md) |
| `src/calliope-lib/` | shared library: stems API types, FLAC parser, process runner, HTTP client | (see its `Cargo.toml`) |
| `src/calliope-stems/` | the edge-AI stem extraction server; runs on archserver | [README](src/calliope-stems/README.md) |

Also at the root: `Cargo.toml` (workspace members only), `Cargo.lock`, `target/` (build output
of all crates), `docs/` (architecture, UI guide, licences) and `specs/` (feature specs, plans
and reports).

## Quick start

```bash
cd src/calliope-gui
npm ci                      # once, and after package-lock.json changes
npm run build:app           # -> ../../target/release/calliope-gui
npm test                    # the whole headless suite (all three crates)
```

The stem server: `cargo build --release -p calliope-stems` (from anywhere in the repository),
then see its README for deployment.

## Licence

Calliope is licensed under the [Apache License, Version 2.0](LICENSE)
(SPDX: `Apache-2.0`). Copyright 2026 Valentin Rusu. The licences of its dependencies and
of the external tools it runs are recorded in [`docs/licences.md`](docs/licences.md).
