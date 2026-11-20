# Pre-PR review rubric

The pr-critic agent reviews every diff against this list before a PR opens.
Each line is a check that a writer has actually gotten wrong. When the user corrects a
PR for something this list misses, add the check here in the same PR — one line, ending
with the spec or PR that earned it. A blocker keeps its `R<n>` id for life; a removed id
is never reused, because the verdict log counts blocks by id.

The seed lines below were earned in the Calligence repository this harness was ported
from; they are stack-agnostic.

## Blockers — the PR does not open until each is fixed or rebutted

- **R1. No hard-coded categorical values.** File types, index states, filter kinds and similar
  closed sets live in an enum or a named constant — never as string literals repeated across
  branches, comparisons or map keys (Rust `match` on strings, TS string unions duplicated
  across files) (inherited).
- **R2. No module-level mutable globals.** No `static mut`, no global `Mutex`/`RefCell`/
  `lazy_static`/`OnceLock` holding state mutated at runtime, no mutable module-level JS
  objects; state goes through a parameter or an owned struct (Tauri managed state counts
  as owned) (inherited).
- **R3. Every new behavior has a test**, and tests cite the `VFS-<n> AC-<m>` ids they prove
  when the spec has acceptance criteria. No `#[ignore]`, `.skip`, `.only`, `todo!()`,
  `unimplemented!()`, stub tests or TODO placeholders (inherited).
- **R4. No test pins a config constant to a hard-coded copy**; tests assert behavior
  (inherited).
- **R5. A `CLAUDE.md`, spec or doc the change makes wrong is edited in the same PR**,
  replacing the rule rather than appending; an implemented spec is not edited — a new spec
  supersedes it (inherited).
- **R6. Committed text is fit for a public open-source repo.** No details about the authors,
  their relatives or their private life in code, docs, specs, fixtures, mockups, commit
  messages or the PR body; users are "the user" / "they", and sample data is invented.
  `scripts/personal_refs.py` catches the obvious phrases in CI; judge the rest (#120).

## Worth flagging — not blocking

- Duplicated logic that an existing function or module already covers.
- Error handling for impossible cases, speculative configuration, abstractions with one caller.
- Changes outside the spec's scope.
