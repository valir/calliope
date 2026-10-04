# calliope

Spec-driven project built by the agent team (see `/build-spec`).

- `specs/overview.md`: the whole system (the owner's document; agents read it, never edit it)
- `docs/architecture.md`: cross-feature technical decisions (maintained by the architect)
- `specs/<feature>.md`: one feature each, plus its `.plan.md`, `.log.md` and `.report.md`
- Agents never touch real external gear or other machines; they use simulators/fakes.

## Conventions
<!-- Language, style, test command, anything every agent should know. The architect
     fills this in on the first build if it's left empty. -->
