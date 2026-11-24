# VFS-43 — Compaction never blocks other writers, and a busy index never stops a content run

- **Status**: implemented (PR pending)
- **Size**: S (a release-blocking bug with a repro; it supersedes VFS-40 R2's merge size and
  priority, D3, and the priority in VFS-40's pre-mortem)
- **Links**: VFS-40 (index compaction), VFS-5 R3 (no write transaction in background mode),
  VFS-2 R20 and VFS-27 R15 (amended in the same PR), e2e run 45487168816 on `main` 3f42238

## Problem

The windows-journey job of e2e run 45487168816 timed out after 900 s, waiting for the content
run to finish. The app log:

```
21:24:56 content run start: 0 of 8957 documents
21:24:58 root_removed id=1 dirs=1711 files=22163 ms=1822
21:25:11 content run stop (IndexWrite): 0 indexed, 0 skipped, 0 failed, 8957 left
21:26:39 compact deleted=100052 slices=2 busy_ms=25209 vacuum=skipped index_mb=43->43
```

The root removal made compaction due, and compaction started right away. VFS-40 ran each merge
slice, and the `VACUUM`, in background mode (very-low I/O priority) while holding `index.db`'s
write lock, against VFS-5 R3. On a busy runner, starved I/O held the lock for about 12 s per
slice; the same work takes 0.45 s on an idle M1. The content run's first write waited out its
10 s busy timeout and failed. `IndexWrite` was a terminal stop that waited for «Reintentar», so
content search stayed stuck: on the target PC, for good.

## Requirements (EARS)

- R1. WHEN the idle indexer compacts (VFS-40 R2), THE indexer SHALL run each merge slice and the
  `VACUUM` at its own priority, never in background mode, and SHALL merge with FTS5's
  `('merge', -16)`. VFS-40's rest of three times each slice's duration is unchanged.
- R2. WHILE another connection holds `index.db`'s write lock past the content run's busy
  timeout, THE content run SHALL wait and try again until the write succeeds or the run is
  stopped, never stop the run (VFS-2 R20).
- R3. WHEN a content run stops because `index.db` could not be written for another reason (an
  I/O error, a full disk), THE app SHALL show the reason with «Reintentar» and SHALL also start
  the run again on its own after 5 minutes (VFS-27 R15).

## Acceptance scenarios

- AC-1 (R1) Given a removal that leaves enough deletions, When the idle indexer compacts, Then
  it writes the `compact` record and never enters background mode.
- AC-2 (R2) Given another connection that takes `index.db`'s write lock at the run's first
  document and holds it for 12 s (longer than the 10 s busy timeout), When the run goes on,
  Then it finishes with every document read and its state mirrored in `index.db`.
- AC-3 (R3) Given every write of a content state failing, When the run stops with `IndexWrite`
  and the writes then work again, Then the service finishes a run on its own, with no
  «Reintentar».

## Non-goals

- Pausing compaction while a content run writes. They are separate subsystems with no channel
  between them. On the target PC, a content run lasts hours, so pausing would postpone
  compaction for that long; with R1 a slice holds the lock for tens to hundreds of
  milliseconds, and R2 waits out a longer hold.
- Changing either connection's busy timeout: 30 s for the indexer, 10 s for the content run.

## Decisions

- D1. `('merge', -16)` instead of `-64`: the same total work, with each lock hold about a
  quarter as long. Merging away 100 k deleted files took 7 slices of at most 136 ms at `-64`,
  and 15 slices of at most 40 ms at `-16`; 449 ms in total either way (M1 SSD, release build).
  On the target PC, VFS-40 D3 expected `-64` slices of about 2–5 s, so `-16` slices should be
  around 0.5–1.3 s **[estimate]**, with the whole job unchanged at about 15–25 minutes, once.
  Step 27m of the target-PC measurement should check it.
- D2. The automatic retry after an `IndexWrite` stop waits 5 minutes: such a stop now means a
  real write failure, and trying again more often would only repeat the failed run start
  (queue rebuild, worker start). It costs nothing between tries, on either machine.
