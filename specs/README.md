# specs/ — change proposals for M- and L-sized work

A spec says what a significant change must do, agreed before we build it. It
lives next to the code so the PR shows it, Claude reads it without any tool,
and tests can cite its acceptance criteria. Work items are GitHub Issues in
`AndresHG/VFS`, grouped in one milestone per phase (Fase 0 Cimientos, Fase 1
Búsqueda por nombre y ruta, Fase 2 Validación en el PC real, Fase 3 Contenido
de documentos, Fase 4 Pulido y distribución); an issue tracks the work, while
the spec folder and the PR body are the durable record of what a change must
do. (Calligence kept specs in a gitignored state folder, which is deleted with
its worktree, and lost specs that committed code still referenced.)

## When a spec lives here

For M- and L-sized changes, as sized by `/spec`: index schema changes, new
platform-specific paths (watcher, shell integration, installer), multi-PR
work, anything that touches user files beyond reading their metadata. S-sized
changes need a repro and the expected behavior in the PR body.

## Ids and layout

Ids are `VFS-<n>`, the next free number above the highest folder here. The
id threads through the branch (`feat/vfs-<n>-<slug>`), commit scopes
(`feat(vfs-<n>): …`) and the doc comments of the tests that prove it.

```
specs/vfs-<n>-<slug>/
  spec.md     requirements (EARS), acceptance scenarios (AC-n), non-goals,
              pre-mortem, decisions; start from
              .claude/skills/spec/template.md
  design.md   optional: architecture fit, data and migration, failure modes,
              test plan
  tasks.md    optional: PR-sized slices, each mapped to its AC ids
```

## Lifecycle

`draft` → `approved` (before any production code) → `implemented (PR #…)`.
The spec stays as the record of intent; only its Status line changes.

## Rules

- Tests cite `VFS-<n> AC-<m>` in their doc comments, so a spec's coverage is
  a grep.
- Don't edit an implemented spec to track later changes: write a new spec
  that supersedes it.
- Never reference `.omc/`, or any other untracked path, from committed files.
- English. Quoted user-facing copy stays Spanish.
