# VFS-33 — A huge root settles: protected folders once, quiet background updates

- **Status**: approved (part 1, after an independent critic pass); part 2 (default exclusions) waits on the owner
- **Size**: M (indexer only, no schema change; part 2 adds a platform-specific list)
- **Links**: issues 159, 156; VFS-1 I11 and U6; VFS-5 R1–R3 (pass modes); VFS-10 R1 (reasons)

## Problem

The owner added their whole home folder (4.87 M files, 597 k folders) on macOS. The first
walk was fine (1,496 s, ~195 k files/min), but afterwards the app never settled. A
`pass_end mode=walk` came every 1–10 s, the status kept saying «Comprobando cambios…» and
`retries_pending=141` stayed put. There are two causes. First, macOS refuses to list ~140
protected folders under `~/Library` and `~/Pictures` (EPERM). VFS-10 R1 keeps such folders
pending and retries them on the I11 schedule. While they are pending they count in the
pass-mode total (141 > `incremental_max_dirs` = 64), so every watcher pass, however small,
runs as a walk, and each attempt logs a warning with the folder's path. Second, every pass
publishes the scanning state, even a one-folder update, so constant small changes keep
«Comprobando cambios» on. The target PC has the same churn in `AppData` and a sync client
that is always on, so it would never look settled either.

## Requirements (EARS)

- R1. WHEN a directory fails to list with "no permission", THE indexer SHALL record its
  reason at once (VFS-10) and retry it once after `RETRY_FIRST` (1 min). This covers denials
  that pass, such as a folder being deleted on Windows or a sync or antivirus filter. IF it
  fails again, THEN the indexer SHALL mark it clean in the same transaction, with no retry
  left. After that it is listed again only when a full walk of its root runs: «Buscar
  cambios ahora», the weekly periodic walk, the root coming back online, or lost change
  history. A restart replays the change history and does not re-check it (VFS-9). This
  amends VFS-10 R1 and AC-1 and VFS-1 I11 for this reason only. Every other reason keeps the
  six I11 attempts.
- R2. WHILE a directory has the reason "no permission", watcher hints and replayed changes
  (VFS-9, VFS-26) SHALL NOT mark it pending. A watcher hint naming the folder itself marks
  its parent only (the folder may be gone). A hint for any path below it, at any depth,
  marks nothing. A replayed change naming the folder marks nothing. A subfolder indexed
  before the denial is not protected itself: a hint there marks it, and its listing fails
  and settles under R1.
- R3. WHEN the indexer picks a pass mode (VFS-5 R1), it SHALL count only pending directories
  that are not waiting for a retry, the same set the pass lists.
- R4. WHILE a batch runs in incremental mode (no root walked in full, at most
  `incremental_max_dirs` folders due when the batch's mode is chosen), THE indexer SHALL NOT
  publish the scanning state. The status and the footer buttons stay as they were
  («Última revisión…», «Buscar cambios ahora»), results still refresh after each commit, and
  a pass that listed anything ends with a fresh idle snapshot, so the counts follow. The mode is chosen per batch, so a burst of changes that pushes the due
  folders over the limit, or a root coming back online, switches to walk mode and shows
  «Comprobando cambios…»; this is intended. This amends VFS-1 U6: «Comprobando cambios…»
  shows during a full walk or a batch of more than `incremental_max_dirs` folders, never for
  a small background update.

## Acceptance scenarios

Tests cite these ids (`VFS-33 AC-<m>`) in their doc comments.

- AC-1 (R1; Unix, mode 000) Given a scanned root with a folder made unreadable, When a full
  walk lists it, Then it has reason "no permission" and counts at once, and is pending with
  one retry. When the retry is due and fails, Then it is clean with no retry entry, and
  running the indexer again lists nothing and counts no error. When another full walk runs
  after its permission is restored, it is listed and its reason is gone.
- AC-2 (R2) Given a folder with reason "no permission", When a watcher hint names a file
  inside it or a deeper path, or a replayed change names it, Then no folder becomes pending.
  When a watcher hint names the folder itself after it was deleted, Then only its parent
  becomes pending, and listing the parent drops the folder and its reason. Given a readable
  folder, a hint marks it pending as before.
- AC-3 (R1) Given a folder failing with any other error (a file where the folder was), Then
  it is still retried on the I11 schedule with growing delays and settled after
  `RETRY_ATTEMPTS` (VFS-1 AC-7, VFS-10 AC-6 unchanged).
- AC-4 (R3) Given more than `incremental_max_dirs` pending folders that are all waiting for a
  retry, and one other pending folder, When the pass mode is chosen, Then it is incremental.
  Without the waiting retries, the same count of due folders chooses walk.
- AC-5 (R4) Given a ready root and an idle indexer, When a small incremental pass lists a
  changed folder after the progress throttle has elapsed, Then the sink receives no scanning
  progress, the index-changed signal still fires, and a fresh idle snapshot arrives. When a
  full walk runs, the sink receives scanning progress as before.

## Non-goals

- Default exclusions of system and app-data folders: part 2, after the owner's decision
  (Decisions #3).
- Coalescing watcher passes for longer than the existing 2 s, or rate-limiting the
  `pass_end` log lines of incremental passes: filed as a follow-up with the measurements
  below.
- Granting macOS Full Disk Access, or explaining it in the UI.
- Retry state still lives in memory (VFS-10 non-goal unchanged).
- «Pausado» after «Continuar»: the paused state stays until the next idle or scanning
  update, as before; an incremental pass lasts milliseconds.

## Measurements (a developer Mac, home folder as the root, v0.2.0, SSD)

- Index 1.64 GB for 4.87 M files + 597 k folders (~337 B per file, FTS included); WAL held at
  64 MB. Commits: 325, avg 44 ms, max 977 ms. Checkpoints: 65, avg 129 ms, max 666 ms.
  Working set: 88–135 MB during the walk (AC-9 budget < 300 MB).
- Of the 4.87 M files, ~3.05 M are under code checkouts and their worktrees (Python venvs,
  Rust `target`, `node_modules`), and 0.73 M are under `~/Library` (Caches 374 k, Application
  Support 240 k). About 31 k are under Documents.
- FSEvents overflowed every few minutes under build churn. Each time it replayed ~5,000
  folders (VFS-26), which takes about 3 s to mark.

## Pre-mortem

- Slow PC, HDD: fewer listings than before (no retry loop); no new I/O.
- Long paths, case: n/a because no path handling changes.
- NFC/NFD: n/a because nothing is matched or folded.
- Antivirus, sync clients: on Windows `ERROR_ACCESS_DENIED` also covers passing cases (a
  folder pending deletion, a filter driver). The one retry after 1 min covers those. A
  denial that lasts longer than that waits for the next full walk, as the VFS-10 list
  already tells the user («Buscar cambios ahora» checks again). That passing denials always
  clear within a minute is an assumption the CI does not verify.
- Killed mid-write: the reason and the clean mark are written in one transaction.
- Million-file match: n/a because search is unchanged.
- Permission-denied: this is the change. Such a folder is listed once per full walk and
  shown once in the list, and hints inside it do not loop.
- User files: none opened or written.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Retry "no permission" folders? | Once, then settle. A TCC or ACL denial does not heal by waiting (on macOS: 140 folders, 4 tries each in 15 min), and the VFS-10 list already says «Buscar cambios ahora» checks again. The one retry covers passing denials on Windows | Claude (issue 159 data, critic) | 2026-11-22 |
| 2 | When does «Comprobando cambios» show? | During walk-mode passes only (full walks and batches over 64 folders); incremental updates are silent | Claude | 2026-11-22 |
| 3 | Default exclusions (OS and app-data trees, developer folders) | Asked through the coordinating session; part 2 | owner | pending |
| 4 | Approval of part 1 | Approved by the session after an independent critic pass, per the owner's standing instruction | Claude | 2026-11-22 |

## Open questions

- Decisions #3 (part 2 only; blocks nothing in part 1).
