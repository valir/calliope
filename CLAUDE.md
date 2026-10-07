# calliope

Spec-driven project built by the agent team (see `/build-spec`).

- `specs/overview.md`: the whole system (the owner's document; agents read it, never edit it)
- `docs/architecture.md`: cross-feature technical decisions (maintained by the architect)
- `specs/<feature>.md`: one feature each, plus its `.plan.md`, `.log.md` and `.report.md`
- Agents never touch real external gear or other machines; they use simulators/fakes.

## Conventions
- Cargo workspace, one folder per crate: `src/calliope-gui/` (desktop app, also the npm
  project), `src/calliope-lib/` (shared library), `src/calliope-stems/` (edge-AI server).
  `Cargo.lock`, `target/`, `docs/` and `specs/` are at the repository root.
- **npm commands run in `src/calliope-gui/`**: `npm test` (whole headless suite, all crates),
  `DISPLAY=:1 npm run test:gui` (GUI tests), `npm run build:app`.
- Path conventions and the full layout: "Paths in this document" and "Repository layout" in
  `docs/architecture.md`.
