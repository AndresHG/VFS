# VFS-10 — Folders that could not be read: a visible list

- **Status**: implemented (PR #138); approved after an independent architect review
- **Size**: L (a schema migration: one column and one partial index on `dirs`)
- **Links**: issue 71; prior-art proposal P7 (`docs/research/prior-art-proposals.md`); VFS-1
  I11 (retries); VFS-25 (cloud-only files, issue 49) and content extraction (Fase 3) will
  reuse the list

## Problem

When a folder cannot be listed (no permission, a sync lock, a cloud-only folder, a failing
disk), VFS retries it (VFS-1 I11) and then waits for the next full walk. The user only sees
results missing, with no explanation. Recoll writes skipped files to a log nobody finds;
Kensaku's technical warnings confused its users. One plain line per folder in the folders
panel, "N carpetas no se pudieron revisar · Ver", with each path and a plain reason, makes a
missing file explainable over the phone.

## Requirements (EARS)

- R1. WHEN a directory fails to list with "no permission", or fails for the last allowed time
  with any other reason (VFS-1 I11, `RETRY_ATTEMPTS`), THE indexer SHALL record on that
  directory its reason from a fixed set (no permission, not available, cloud-only, disk
  error), in the same transaction as the failure. A "no permission" folder is still retried
  on the I11 schedule. A recorded reason is replaced only under the same rule (a later "no
  permission" failure, or a later exhausted run), never on an intermediate failure. A root's
  own folder is recorded like any other.
- R2. WHEN a directory with a recorded reason is listed successfully, or is removed from the
  index (deleted, its root removed), THE indexer SHALL forget its reason in the same
  transaction. Marking a directory pending again (a re-walk, a watcher hint) SHALL keep it.
- R3. THE reason SHALL come from the listing error alone, with no extra file-system call:

  | Reason | `io::ErrorKind` | Windows `raw_os_error` | Unix `errno` |
  |---|---|---|---|
  | no permission | `PermissionDenied` | (5, through the kind) | (EACCES, EPERM, through the kind) |
  | not available | `TimedOut`, `HostUnreachable`, `NetworkUnreachable`, `NetworkDown`, `StaleNetworkFileHandle` | 21 `ERROR_NOT_READY`, 53 `ERROR_BAD_NETPATH`, 55 `ERROR_DEV_NOT_EXIST`, 59 `ERROR_UNEXP_NET_ERR`, 64 `ERROR_NETNAME_DELETED`, 67 `ERROR_BAD_NET_NAME`, 121 `ERROR_SEM_TIMEOUT`, 1231 `ERROR_NETWORK_UNREACHABLE` | ENXIO, ENODEV, EHOSTDOWN |
  | cloud-only | — | every `ERROR_CLOUD_FILE_*` code of `winerror.h` (358–475, not contiguous), listed by name; on Windows a test checks each value against `windows-sys` | — |
  | disk error | anything else | | |

  `NotFound` never reaches this mapping: it removes the folder or makes the root unavailable
  (VFS-1).
- R4. A directory SHALL count as unreadable while it has a reason. WHEN the roots are listed, THE core SHALL give each root its number of unreadable
  directories (one query on a partial index, `dirs_unreadable ON dirs(root_id) WHERE error IS
  NOT NULL`); WHEN asked for one root's unreadable directories, it SHALL return each one's id,
  absolute path and reason, at most 500, in path order.
- R5. WHILE a root has unreadable directories, THE folders panel SHALL show under that root
  "1 carpeta no se pudo revisar · Ver" / "N carpetas no se pudieron revisar · Ver"; "Ver"
  SHALL open a list with each path, its plain reason and "Mostrar en carpeta", a line "y N
  más" when the count exceeds the list, and the hint that after fixing access «Buscar cambios
  ahora» checks again.
- R6. THE index SHALL migrate from the previous schema version without a rescan; the reason
  SHALL be cleared in the `UPDATE` that marks a listed folder clean (`apply::apply_listing`'s
  last statement, through its own helper, not the shared `set_dir_state`), adding no
  statement per successful listing and no per-file work; recording costs one `UPDATE` per
  failed listing.

## Acceptance scenarios

Tests cite these ids (`VFS-10 AC-<m>`) in their doc comments.

- AC-1 (R1, R4; Unix, mode 000) Given a root with one folder the user cannot read, When it is
  scanned, Then that folder has reason "no permission" and counts at once, the root's count is
  1, and its list holds the folder's path and reason; the folder is still pending and retried
  on the I11 schedule; the readable folders are indexed as usual. Windows coverage of R1 rests
  on AC-3 and AC-6.
- AC-2 (R2) Given a counted folder (any reason) still unreadable, When it is marked pending
  again (a re-walk) and fails again, Then it keeps its reason and still counts; When its permission is restored and it is listed,
  Then the reason is gone and the count is 0; Given a recorded folder, When it is deleted from
  disk and its parent is listed, or its root is removed, Then no reason remains.
- AC-3 (R3) Unit test: one error per row of the table maps to its reason, including the
  Windows and Unix raw codes and a cloud code at each end of the range.
- AC-4 (R5) UI test: the line has a singular and a plural form in both catalogs and is
  hidden for 0; every reason has its plain Spanish and English text; the "y N más" line.
- AC-5 (R6) Given an index at the previous schema version, When opened, Then it has the
  column and the partial index, no directory has a reason, and no rescan is queued.
- AC-6 (R1, R4) Given a folder whose listing fails with a disk error, When it fails before
  its retries are exhausted, Then it has no reason and does not count; When the retries are
  exhausted, Then it has reason "disk error" and counts.

## Non-goals

- No list of individual files that could not be read; folders only (content extraction
  failures come with Fase 3).
- Transient failures that succeed on a retry are never shown: only "no permission" counts
  before the retries are exhausted (R4).
- Retry attempts stay in memory (VFS-1 I11): a disk, network or cloud error in a folder of an
  app that is always closed before the retries end (about 31 minutes) is never counted.
  Accepted for v0.2.0; "no permission", the common case, counts at once.
- No retry button in the list: "Buscar cambios ahora" already walks again.
- Root folders that are unreachable keep their existing "no disponible" status; they are not
  listed here.

## Pre-mortem

- Slow PC, multi-TB HDD: no extra I/O; one `UPDATE` per failed listing records the reason,
  and the `UPDATE` that already marks a listed folder clean clears it (R6). Counting uses a partial index holding only unreadable folders.
- Long paths and case: paths are built from `roots.path` and `dirs.rel_path` like every other
  path (`db::join_rel`); "Mostrar en carpeta" uses the existing reveal (VFS-1).
- NFC/NFD: n/a because nothing is matched or folded; paths are shown as stored.
- Antivirus: a folder Defender keeps locked past the retries is listed as "error de disco"
  or "sin permiso" by its error; it clears on the next successful listing.
- Killed mid-write: the reason is written in the indexer's transactions; a crash loses at
  most an uncommitted reason, written again by the next "no permission" failure or the next
  exhausted run (see the non-goal on retries kept in memory).
- Million-file match: n/a because search is unchanged; the list is capped at 500.
- Permission-denied, links, cloud placeholders: exactly what is recorded. A folder whose
  listing fails because one entry's metadata is denied (VFS-3, #56, `walk::list_dir`) is
  shown as "sin permiso" although most of it can be opened; acceptable, the path is the right
  place to look. Links are never followed (VFS-18), so a link is never an unreadable folder.
- User files opened or written: none; only the existing listing.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Where does the reason live? | A nullable `dirs.error` column (`ALTER TABLE … ADD COLUMN`: no table rewrite; at most one header byte on a row when it is next written) and a partial index `WHERE error IS NOT NULL`; a separate table would need a delete on every listing | Claude | 2026-11-21 |
| 2 | One global line or one per folder? | One per chosen folder, under it in the folders panel, as issue 71 describes | Claude (issue 71) | 2026-11-21 |
| 3 | Reasons | The four of P7: "sin permiso", "no disponible", "solo en la nube", "error de disco" | owner (issue 71, approved proposal) | 2026-11-21 |
| 4 | When does a failing folder show? | "No permission" at once; other reasons once the retries are exhausted (P7's "list only after the retries"); a reason is written only when it should show and kept until a successful listing, so a re-walk never hides it | Claude (architect review) | 2026-11-21 |
| 5 | Approval | Approved by the session after an independent architect review of this spec, per the owner's standing instruction for v0.2.0 | Claude | 2026-11-21 |

## Open questions

None.
