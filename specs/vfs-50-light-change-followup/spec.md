# VFS-50 — The work after a change is proportional to the change

- **Status**: in progress
- **Size**: M (the content run's queue and mirror, its trigger, and the name pass's root
  counts; one additive index in `content.db`, no version change; no `index.db` schema change)
- **Links**: VFS-2 R3, R20 (queue rebuild, mirror), VFS-27 R7, R8, R10 (pace, progress, new
  documents), VFS-33 R4 (small name updates leave the status alone), VFS-1 AC-7 (watcher)

## Problem

The owner sees VFS "always working": «Leyendo documentos…» shows again and again, and the
process reads gigabytes while only a few files change. Measured on the owner's Mac (236 k
files, 29 k documents, roots Documentos and Github; 2026-11-26):

- Watching costs nothing measurable; one incremental name pass lists only the changed folders.
- Every name change with a commit pokes the content service. A content run then rebuilds the
  queue (`enqueue`) by scanning every row of `c.docs`, whose rows hold the compressed text:
  253 MB read for a check on a few columns (1.18 s).
- The mirror then walks `files` in windows of 5,000 ids, and each window's
  `id IN (SELECT file_id FROM c.docs)` is planned as a scan of all of `c.docs`: work grows with
  windows × documents. 49 windows × 29 k documents took 2.17 s warm; bounding the subqueries
  to the window took 0.05 s. On the target PC (5 M files, ~0.5 M documents, HDD) that is
  ~1,000 windows each reading a ~4 GB table **[estimate]**: the run never looks finished.
- 76 runs in 3 days, 15 with nothing to read; a run for 2 documents took 1–16 s and always
  showed «Leyendo documentos: N de N».
- Each incremental pass of a ready root recounts all its files (`refresh_counts`).
- After a finished run, the service recomputes a signature over all of `files` every 10
  minutes even when nothing changed.
- Process totals in 2 h 14 min of the owner's normal use: ≈10 GB read, 110 s CPU.

The owner chose the hybrid (2026-11-26, project chat card): keep watching, keep names live,
read changed documents in calm moments, and show no «processing» for small updates.

## Requirements (EARS)

- R1. WHEN the content run mirrors states into `index.db`, THE run SHALL bound every
  subquery on `c.docs` and `c.queue` to the window's file ids, so one mirror reads each
  store row at most once.
- R2. WHEN the content store is opened, THE store SHALL have an index on `docs` covering the
  columns the queue rebuild compares (`docs_meta`), created if missing, without changing
  `STORE_VERSION`; the queue rebuild SHALL read `c.docs` through it.
- R3. WHEN the name indexer reports changed names, THE content service SHALL start a run only
  after 2 minutes with no further report, and at most 30 minutes after the first report it
  has not yet acted on. A switch-on, resume, «Reintentar», «Volver a leer» or app start SHALL
  start at once, as before.
- R4. WHEN a run starts with at most 20 documents to read, THE service SHALL leave the shown
  status as it was (phase and counts) until the run ends, then publish the finished status
  once. A run with more documents shows «Leyendo documentos» as before (VFS-27 R8).
- R5. WHILE the last run finished and no name change has been reported, THE service SHALL
  not recompute the documents signature, except every 10 minutes while a timed-out document
  waits for its one more try (VFS-2 R3).
- R6. WHEN an incremental pass ends on a root that is ready, THE indexer SHALL update the
  root's file count by the pass's added minus removed files and its folder count from the
  folder index, instead of recounting its files; the end of a full walk still recounts.
- R7. WHEN a content run stops, THE diagnostic record SHALL also carry the milliseconds spent
  rebuilding the queue and mirroring (`prep_ms`).

## Acceptance scenarios

Tests cite these ids (`VFS-50 AC-<m>`) in their doc comments.

- AC-1 (R1) Given a store with documents spread over many mirror windows, When a run
  mirrors, Then every file's `content_state` matches the store exactly as before, and the
  mirror's query plan searches `c.docs` by rowid range, never a full scan per window.
- AC-2 (R2) Given an existing version-2 store without `docs_meta`, When it is opened, Then
  the index exists, the version is still 2, nothing is rebuilt, and the stale-row query's
  plan scans `docs_meta`.
- AC-3 (R3) Given a finished run, When names change twice 60 s apart, Then no run starts
  before 2 minutes after the second change, and one run starts after it.
- AC-4 (R3) Given names changing every 30 s without pause, When 30 minutes have passed since
  the first change, Then a run starts.
- AC-5 (R4) Given a finished run and 3 new documents, When the run reads them, Then no status
  event with phase `reading` or `waiting` is sent, and one event with phase `finished` and
  the new counts is sent at the end.
- AC-6 (R4) Given 50 new documents, When the run starts, Then the status shows `reading`.
- AC-7 (R5) Given a finished run and no timed-out document, When 25 minutes pass with no name
  change, Then the signature is not computed.
- AC-8 (R6) Given a ready root, When an incremental pass adds 3 files and removes 1, Then its
  file count grows by 2 without a recount, and a later full walk's recount agrees.
- AC-9 (R7) The `content run stop` record has `prep_ms=<n>`.

## Non-goals

- A refresh button or stopping the watcher: watching costs nothing measurable (owner chose
  the hybrid).
- Telling the content service which names were documents: R3's wait already bounds how often
  a signature or queue rebuild runs; a follow-up may add it if R7's numbers ask for it.
- Feeding the queue with only the changed files: with R1 and R2 a rebuild reads the files
  table and an index of a few bytes per document.
- The interface's re-run of the visible search on `index-changed`.
- Index compaction not starting on the owner's Mac (1.9 GB for 236 k files): its own fix.

## Decisions

- D1. 2 minutes of calm, 30 at most: a document saved and closed is searchable by its words a
  few minutes later (by name at once); a sync burst on the target PC costs one run, not one per
  file. **[estimate]** to be checked with R7's records on the target PC.
- D2. 20 documents for a silent run: about 10 s of reading on the target PC (VFS-27 D3), shorter
  than anyone would watch for.
- D3. An index instead of moving the text to its own table: same effect on the rebuild, no store
  version change, so no re-reading of anyone's documents.
