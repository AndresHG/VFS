---
name: spec
description: Size a change, clarify it with the user, and agree its acceptance criteria before any code is written. Use for any request to build or change VFS behavior, when picking up a VFS-<n> spec, and when the user asks to spec, scope, clarify, "interview me" or plan a change. Input is a VFS-<n> or a description.
---

# /spec — size, clarify, specify before code

Turn `$ARGUMENTS` (a `VFS-<n>` or a description) into an agreed, testable
statement of the change. This skill writes no production code: it ends at an
approved set of acceptance criteria.

VFS has no ticket system. Ids are `VFS-<n>`: the next free number above the
highest folder in `specs/`. The spec folder and the PR body are the durable
record.

## 1. Research before asking

- Read the specs the change touches (`specs/README.md` lists the layout) and
  the code entry points they name.
- Write a short "what I know" note: facts with citations (paths, specs, PRs),
  then the assumptions you are making.
- Never ask the user something the code or a spec answers. Resolve it
  yourself and cite where.

## 2. Size the change, out loud

| Size | Typical | Output |
|------|---------|--------|
| S | Bug with a repro, copy, config, dependency bump; the diff fits in one sentence | Repro + expected behavior in the PR body, then a failing test |
| M | New or changed behavior inside one subsystem, one PR | `specs/vfs-<n>-<slug>/spec.md` with only Problem, Acceptance scenarios, Non-goals, Decisions |
| L | Index on-disk schema change, new platform-specific path (watcher, shell integration, installer), multi-PR, or anything that touches user files beyond reading their metadata | `specs/vfs-<n>-<slug>/spec.md` from the full template |

- Anything that changes the SQLite schema, reads outside the selected root
  folders, opens or writes user files, or behaves differently on Windows and
  macOS is at least M. A schema change that needs a migration is L.
- When unsure, pick the larger size. Discovering more complexity later moves
  the change up a size, never down.
- State the size and one line of reasoning; the user can override it.
- S stops after the repro and expected behavior are written down.

## 3. Interview (M and L)

1. List the unknowns. Tag each: blocking, safe to default, or answerable from
   code (resolve that kind yourself).
2. Ask through `AskUserQuestion`, at most 4 questions per round, highest
   leverage first. Each question gets 2–4 concrete options with your
   recommendation first, and says what each option implies.
3. Walk [premortem.md](premortem.md). Every item is answered or marked
   "n/a because …".
4. Log every answer: question, choice, who, date.
5. At most 5 rounds. If something is still ambiguous, write it down as a known
   risk and ask whether to proceed with it.

## 4. Write the acceptance criteria

- Requirements in EARS form: `WHEN <trigger>, THE <component> SHALL
  <response>` (also `WHILE <state>` and `IF <condition>, THEN …`).
- Acceptance scenarios with stable ids `AC-1`, `AC-2`… in Given / When /
  Then. Each one is observable and becomes at least one test.
- Non-goals: what this change deliberately does not do.
- Copy [template.md](template.md) to `specs/vfs-<n>-<slug>/spec.md` and fill
  it in (M: the four sections named above). See `specs/README.md`.

## 5. Gate: stop for approval

Stop when all of these hold:
- every requirement has an observable check;
- the non-goals are written down;
- no open question would change an interface, the index schema or a
  migration;
- no requirement can be read two ways.

Show the artifact and ask the user to approve it with `AskUserQuestion`. Only
after approval: plan (M: plan mode; L: `design.md` + `tasks.md` in the spec
folder), then implement test-first. Every test cites the `VFS-<n> AC-<m>` ids
it proves in its doc comment.

## Rules

- Ask only what only the user can answer.
- A few sharp acceptance criteria beat twenty vague ones; an M change
  rarely needs more than five.
- Files under `specs/` are English; quoted user-facing copy stays Spanish.
- If a question this skill should have asked later causes a bug, add it to
  `premortem.md` with the spec or PR that taught it, in the same PR as the
  fix. That is the harness ratchet, applied to clarification.
