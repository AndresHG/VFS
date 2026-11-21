# VFS-23 — Robustness follow-ups: root list backup, backslash names, long words

- **Status**: approved (after the implementation was drafted; reviewed by an independent spec
  critic under the owner's standing instruction, D4)
- **Size**: M
- **Links**: VFS-3 (`specs/vfs-3-robustness/spec.md`), VFS-1 S6; issues 53, 57, 58, 103

## Problem

The VFS-3 harnesses left four phase-1 issues open:

- Issue 58: recovery re-adds the folders it can read from the damaged index. If the page that
  holds `roots` is destroyed, or the file is truncated or its header overwritten, the index
  comes back empty and the user has to add every folder again.
- Issue 57: the query turns `\` into `/` on every platform. A macOS name can hold a backslash,
  so a name with two or more is not found by its exact text.
- Issue 103: damage SQLite cannot detect can leave FTS entries for files that no longer exist.
- Issue 53: one very long pasted word hung the typo tolerance. It is already fixed on main;
  this spec records the criterion.

A root "being removed" is one the user asked to remove whose rows the indexer has not deleted
yet (status `removing`). "The kept roots" are all the others.

## Acceptance scenarios

- AC-1 (issue 58) Given an index with roots, When its first page is overwritten, the file is
  truncated to nothing, or a random page or WAL block is torn so that SQLite reports damage
  (the page holding `roots` included), Then the next open recovers and holds exactly the kept
  roots. After any root is added or removed, the backup lists exactly the kept roots; a failed
  backup write never fails the root change (it is logged), and the next open repairs it.
- AC-2 (issue 57) Given a file named `dos\barras\invertidas.txt` (two backslashes) on macOS,
  When its exact name is searched, Then it is found as an exact name match, also in an index
  built before this change (file name keys were never folded). On Windows, a backslash in a
  query still separates folders.
- AC-3 (issue 53) Given a 10,000-character word without spaces, When it is searched, Then the
  query returns in under 10 s on CI: words over 64 characters (Unicode scalar values, after
  folding) get no typo tolerance at all, and are matched exactly.

## Non-goals

- Cleaning FTS entries left by damage SQLite cannot detect (D2).
- Making root `path_key`s keep a literal backslash on macOS. Stored keys are unchanged, so no
  reindex is needed.
- Cleaning a temporary backup file left by a kill between its write and its rename (a few
  bytes, overwritten by the next write from a process with the same id).

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Root list backup (issue 58) | `<index>.roots`: UTF-8 paths of the kept roots, each ended by a NUL (a path may hold a newline, never a NUL). Written to a per-process temporary file and renamed, after every root change and on each open whose list differs. Read only by a recovery (a damaged index), when `roots` cannot be read or reads as empty; a healthy open never takes roots from it. An index file of 0 bytes with no WAL content, while the backup lists roots, counts as damaged. Only if damage coincides with a kill between removing the last root and writing the backup can that root come back, pending a scan; the user removes it again. The name starts with `index.db`, so the one-time move out of roaming app data carries it (`legacy_index_is_moved_once`) | lead | 2026-11-21 |
| D2 | Stale FTS entries after undetectable damage (issue 103) | Accepted. Only bit rot or a disk that lies about writes causes them. Search stays correct because candidates are checked against `files`; each stale entry costs one candidate read. Rebuilding the FTS of 5 M rows on the target HDD costs far more | lead | 2026-11-21 |
| D3 | Backslash in queries (issue 57) | A separator on Windows only, where no name can hold one; literal on macOS. Supersedes VFS-1 S6's "`\` is folded to `/`" | lead | 2026-11-21 |
| D4 | Approval | The owner's standing instruction: a spec is approved once an independent critic, given only the spec, confirms it makes sense | owner | 2026-11-21 |
