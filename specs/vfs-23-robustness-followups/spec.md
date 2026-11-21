# VFS-23 — Robustness follow-ups: root list backup, backslash names, long words

- **Status**: approved
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

## Acceptance scenarios

- AC-1 (issue 58) Given an index with roots, When its first page is overwritten or the file is
  truncated to nothing, Then the next open recovers and keeps every root that is not being
  removed. After any root is added or removed, the backup list equals that root list.
- AC-2 (issue 57) Given a file named `dos\barras\invertidas.txt` (two backslashes) on macOS,
  When its exact name is searched, Then it is found as an exact name match. On Windows, a
  backslash in a query still separates folders.
- AC-3 (issue 53) Given a 10,000-character word without spaces, When it is searched, Then the
  query returns promptly: typo tolerance stops at 64 characters.

## Non-goals

- Cleaning FTS entries left by damage SQLite cannot detect (D2).
- Making root `path_key`s keep a literal backslash on macOS. Stored keys are unchanged, so no
  reindex is needed.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Root list backup (issue 58) | `<index>.roots`: UTF-8 root paths, each ended by a NUL (a path may hold a newline, never a NUL). Written to a per-process temporary file and renamed, after every root change and on each open whose list differs. Recovery reads it when `roots` cannot be read or reads as empty. An index file of 0 bytes with no WAL content, while the backup lists roots, counts as damaged. Roots being removed are left out of both. The name starts with `index.db`, so the one-time move out of roaming app data carries it | lead | 2026-11-21 |
| D2 | Stale FTS entries after undetectable damage (issue 103) | Accepted. Only bit rot or a disk that lies about writes causes them. Search stays correct because candidates are checked against `files`. Rebuilding the FTS of 5 M rows on the target HDD costs far more than the extra reads | lead | 2026-11-21 |
| D3 | Backslash in queries (issue 57) | A separator on Windows only, where no name can hold one; literal on macOS. Supersedes VFS-1 S6's "`\` is folded to `/`" | lead | 2026-11-21 |
