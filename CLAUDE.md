# VFS — local file search for huge personal archives

## What this is

A desktop app for Windows and macOS that finds any file in a multi-TB personal archive (designed
for 5 M files) by file name and folder path, with filters, typo and accent tolerance, and a
"mostrar en carpeta" link that reveals the file in Explorer/Finder. Fully offline. The user is
non-technical; the UI is Spanish. Product spec: `specs/vfs-1-core-search/spec.md`.

## Tech stack

- **Core**: Rust, cargo workspace.
- **Index**: SQLite in WAL mode, FTS5 over accent-folded names and paths.
- **App**: Tauri 2 with a small web frontend.
- **Targets**: Windows 10+ and macOS; no administrator rights. CI runs a Windows + macOS matrix.
- **Harness**: Python 3 + bash hooks and scripts, run only on the macOS dev machine.

## Build and test

Rust is not on agents' `PATH`: prefix cargo commands with `source "$HOME/.cargo/env" &&`.

- Build: `cargo build --workspace`; CLI: `cargo run -p vfs-cli --`.
- Test all: `cargo test --workspace`; one crate: `cargo test -p vfs-core`.
- Lint: `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- The desktop app and frontend commands are added here when `apps/desktop` lands.

## Git

- `main` is protected. Never commit or push to it directly: branch `feat/vfs-<n>-<slug>` (or
  `feat/<title>` for S-sized work) and open a PR. PRs are squash-merged; history stays linear.
- An auto-created worktree starts on `claude/<name>`: rename it (`git branch -m feat/<title>`)
  before the first push. GitHub cannot re-point a PR's head.
- Never bare `git stash` / `pop` / `clear`: the stash stack is shared by every worktree and
  session. Use a WIP commit, or `git stash push -u -m "<tag>"` then `git stash apply <sha>`.
- Force-push only your own branch, with `--force-with-lease=<branch>:<sha>`. Never repair a
  divergence with a merge commit.
- Commits are conventional with the spec id as scope: `feat(vfs-1): …`, `fix(vfs-3): …`.
- **Changed code ends in a PR, unasked**: commit, run the `pr-critic`, push, open the PR. This
  is standing authorization. Merging and pushes to `main` are always the user's call.
- `guard-git-danger.py` enforces this: it asks before pushes to `main` and PR merges, and
  denies force-pushes to `main`, `claude/*` pushes, bare stash, and `gh pr create` without a
  critic verdict for HEAD.

## Plan before coding: spec first

- Before any implementation, run **`/spec`**: it sizes the change S / M / L, researches the
  code and specs before asking, interviews on what only the user can answer (with a
  pre-mortem), and writes acceptance criteria.
  - **S** (bug with a repro, config, copy): repro in the PR body, then a failing test.
  - **M** (one subsystem, one PR): short `specs/vfs-<n>-<slug>/spec.md`.
  - **L** (index schema, platform-specific path, installer, multi-PR): full spec from
    `.claude/skills/spec/template.md`. When unsure, pick the larger size.
- No ticket system: ids are `VFS-<n>`, the next free number in `specs/`. The spec and the PR
  body are the durable record. See `specs/README.md`.
- Then list the affected components, the tests needed (new + existing) and the approach. Only
  then write code.

## Testing

- Every new behavior has a test. Tests cite the ids they prove in a doc comment
  (`/// VFS-1 AC-10: …`), so a spec's coverage is `grep -rn "AC-10"`.
- S-sized work starts from a failing test.
- Before a PR: the change's own tests, `cargo fmt --check` and the CLAUDE.md budget check
  (`python3 scripts/claude_md_budget.py`). CI runs the full suite on both platforms.

## Review and PR

- A reviewer shares nothing with the writer but the diff and a written rubric. After
  committing, spawn the `pr-critic` agent with the spec id (or acceptance criteria) and the
  base branch, never the conversation. It records a verdict for HEAD; a new commit needs a new
  verdict. Docs-only changes skip it, except agent-steering files (`CLAUDE.md`, `SKILL.md`,
  `.claude/`).
- The rubric is `.claude/review-rubric.md`. When the user corrects a PR for something it
  misses, add one line there in the same PR.
- Only when the user explicitly skips review: `python3 scripts/pr_critic_verdict.py record
  waived "<user's words>"`.
- The PR body follows `.github/pull_request_template.md` and always carries `## Lessons`: each
  correction or miss and where it went, or `None`. `require-pr-on-stop.py` checks this.

## Keep CLAUDE.md files current

- Nested `CLAUDE.md` files hold per-directory rules and load lazily; this file holds repo-wide
  rules only.
- **Replace, don't append.** When a change alters a rule, edit it in place or delete it. The
  file is a sheet of rules that hold today, never a log. What changed goes in the PR body.
- Every bullet is one present-tense rule someone would otherwise get wrong, ending with the
  spec or PR that earned it. No per-file descriptions, no narrative, no dates.
- Budgets live in `scripts/claude_md_budget.py`: a cap per file, a max line length, caps that
  only go down, and no PR grows a file into the last 1 KB under its cap. A hook reports
  breaches on edit.

## The harness ratchet

A failure a control could have prevented gets the control added, not just the instance fixed,
routed by cost of recurrence:
- **Dangerous if repeated** (lost history, cross-session damage, user files) → a
  `.claude/hooks/` guard at the tool boundary.
- **Mechanically checkable** → a test or CI check.
- **Needs judgment** → a rule line in the right `CLAUDE.md` or the rubric. Every line traces to
  a real failure or a hard external constraint; prune lines that trace to nothing.
- **Context, not a rule** → the spec or auto-memory.

Sensors report failures only: `lint-on-edit.py` runs `rustfmt --check` on each edited `.rs` and
is silent on success or when Rust is not installed. Hooks fail open.

## Code

- Check whether a function exists before writing one (`grep` first); check signatures before
  calling.
- Simple over clever; no abstractions for one caller; nothing for hypothetical requirements.
- Code, identifiers, comments and specs are English; user-facing copy is Spanish.
- Comments state behavior and constraint and cite the spec, never who asked.
- Never reference `.omc/` or any other untracked path from committed files.
