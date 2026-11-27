# VFS-53 — «Relacionados»: similar documents by meaning

- **Status**: approved (owner, 2026-11-26; scope narrowed 2026-11-27, D7).
- **Size**: L. It adds a table to `vectors.db` (schema 2 → 3, kept vectors untouched).
- **Links**:
  - [VFS-29](../vfs-29-hybrid-search/spec.md): the e5-small vector set this spec reuses
    (R6, R10, R15, D16, D19, D20), «relacionado por tema» (R12).
  - [VFS-49](../vfs-49-chat-tab/spec.md): the Chat tab and its source panel (R8).
  - [VFS-50](../vfs-50-light-change-followup/spec.md): work after a change follows the change.
  - Research report «VFS: automatic knowledge graph» (G1) and the plan of 2026-11-26.

## Problem

VFS finds a document when the user can name it or its words, but an archive is also a web of
documents about the same thing: the dentist's invoice, the insurer's letter about it and the
Spanish and English copies of a blood test. The owner wants «Relacionados» (similar documents)
inside Buscador and Chat, as one more way to search: VFS searches and answers, and learns
nothing about the archive (D7). Search by meaning already
holds one e5-small vector per chunk text (VFS-29), computed at a cost of 20–35 h of paced CPU
per 60,000 documents on the slow target PC. «Relacionados» must reuse those vectors and add almost
nothing to that cost, on a slow PC with a spinning disk too.

## Requirements (EARS)

Document vectors

- R1. WHEN an embedding run completes the vector set (VFS-29 R6, `complete`), THE run SHALL
  store, for every document with chunks, a document vector: the mean of its chunk vectors,
  minus the set's mean (D19), scaled to unit length; kept as 1-bit and int8 rows like chunk
  vectors, in a `docs` table of `vectors.db`.
- R2. THE run SHALL recompute only the documents whose list of chunk texts changed since the
  stored row (a signature of their chunk hashes), and drop the rows of documents gone; an
  unchanged archive SHALL write nothing (VFS-50).
- R3. THE schema change SHALL keep every stored vector: a version-2 store gains the `docs`
  table and is filled by the next completed run, with no embedding redone.

«Relacionados»

- R4. WHILE search by meaning serves (VFS-29 R11), THE «Documentos» tab SHALL offer
  «Relacionados» on each document hit, and the Chat source panel SHALL offer
  «Ver relacionados» on each cited document.
- R5. WHEN the user asks for the related documents of a document, THE app SHALL return up to 5
  other documents whose document vectors are closest to it, best first, each with a similarity
  of at least [`MIN_SIMILARITY`] (D2), found by a scan of the 1-bit document rows and a rescore
  of the closest with their int8 rows. It SHALL compute them on demand: nothing is stored, no
  thread starts and nothing stays in memory after the answer.
- R6. EACH related document SHALL show why: «mismo tema», plus «misma carpeta» when it is in
  the same folder, and its passage closest to the asked document as the fragment (its origin),
  with «Mostrar en carpeta» as on any hit.
- R7. IF the document has no document vector yet (not read, or embedded after the last
  completed run) or nothing passes the threshold, THEN THE list SHALL say so in one line, never
  show documents below the threshold.

- R8. (Removed: «Temas» left VFS, D7.)

Footprint

- R9. With search by meaning off, this spec SHALL add no file, table, thread or memory. With
  it on, «Relacionados» SHALL answer within 50 ms for 60,000 documents on the development Mac
  (scan and rescore, excluding the first read of the rows from disk), and the document rows
  SHALL take about 0.5 KB per document in `vectors.db` (≈ 26 MB at 60,000 documents).

## Acceptance scenarios

Tests cite these ids (`VFS-53 AC-<m>`) in their doc comments.

- AC-1 (R1) Given a completed run over documents A (two chunks) and B, When it ends, Then `docs`
  holds one row for A and one for B, and A's int8 row is the centred, normalised mean of its two
  chunk vectors.
- AC-2 (R2) Given a completed run, When a second run finds the archive unchanged, Then no `docs`
  row is written; When one document changes, Then only its row is rewritten; When a document is
  removed, Then its row is gone.
- AC-3 (R3) Given a version-2 `vectors.db` with a complete set, When it is opened, Then its
  vectors are kept, the `docs` table exists and the next completed run fills it without
  embedding any text.
- AC-4 (R5, R7) Given documents about two different subjects, When related documents of one are
  asked, Then the documents of its subject come first, a document below the threshold is never
  returned, and the asked document is not in its own list.
- AC-5 (R6) Given a related document with three chunks, When it is returned, Then its passage is
  the chunk closest to the asked document.
- AC-6 (R9) Given 60,000 random document rows, When related documents are asked, Then the answer
  takes under 50 ms in a release build (logged by the test, asserted at a lenient bound in debug).
- AC-7 (R4, R6) Given the «Documentos» tab with search by meaning on, When the user opens
  «Relacionados» on a hit, Then up to 5 documents show with «mismo tema», «misma carpeta» for
  one in the same folder, and «Mostrar en carpeta»; with search by meaning off, no
  «Relacionados» button shows.
- AC-8 (R4) Given a Chat answer citing a document, When the user opens the source and presses
  «Ver relacionados», Then the same list shows in the source panel.
- AC-9 (R9) Given search by meaning off, When the app runs, Then `vectors.db` is not created and
  no related command opens it.

## Non-goals

- A graph view or canvas (owner, 2026-11-26).
- «Temas» (topic groups), people and places, and anything that learns from the archive or
  from Chat answers: out of VFS's scope (D7).
- Expanding Chat retrieval with neighbours (graph-expanded retrieval): later, measured first.
- Related bookmarks, chats or name-only files: only documents with vectors.
- Storing related lists: they are computed on each request (R5).

## Pre-mortem

- Slow PC and HDD: the document rows are built from int8 rows already in `vectors.db`, read
  in hash order per batch of documents, once after the first completed run and then only for
  changed documents; «Relacionados» reads the 1-bit document rows (≈ 56 B each, 3.4 MB at 60,000
  documents) on each request.
- Long paths, NFC/NFD: n/a because rows key on `file_id`, and paths come from the index.
- A root offline or paused: its documents keep their rows until a completed run drops them, and
  a related document no longer in the index is skipped.
- Antivirus: no new file; one more table in `vectors.db`.
- Crash mid-write: each batch of document rows commits in one transaction; a run stopped before
  the end leaves the rest for the next completed run (R2).
- A million matches: the list is capped at 5; the scan is one pass over the document rows.
- Permission-denied, links, placeholders: n/a because only documents already read have vectors.
- Opening, writing, moving or deleting user files: no.
- Does any of this already ship? `grep -rn "related\|docs" crates/vfs-semantic` finds no
  document vectors; VFS-29 ranks chunks only.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Where it shows | Inside Buscador and Chat; no separate graph view | owner | 2026-11-26 |
| D2 | When a document counts as related | Cosine of the centred document vectors ≥ 0.35 (`MIN_SIMILARITY`). On the 29b corpus (3,192 documents, e5-small): random document pairs have median 0.01, 99th percentile 0.30, 99.9th 0.47; 0.35 lets 0.47 % of random pairs through, and 92 % of documents still get a neighbour (4.0 of 5 on average). Uncentred cosines bunch at 0.74–0.87 and cannot separate them | agent | 2026-11-27 |
| D3 | Document vectors stored or on demand | Stored, built after each completed run for changed documents only: averaging a document's chunks on each request would read every chunk's int8 row of every candidate | agent | 2026-11-27 |
| D4 | Related lists stored | No: one scan of 0.5 KB rows per request is cheaper than keeping lists current | agent | 2026-11-27 |
| D7 | Scope | VFS searches and answers; it does not grow or learn. «Relacionados» stays (it is a query by similarity); «Temas», people and places, and Chat answers growing a graph are out of VFS's scope | owner | 2026-11-27 |

## Open questions

None.
