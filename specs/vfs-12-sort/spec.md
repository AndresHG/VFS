# VFS-12 — Sort control: relevance, newest, oldest, name, size

- **Status**: approved
- **Size**: M. One subsystem (the order of results) plus its select on screen. It changes the
  search contract (`SearchQuery.sort` in, `approximateOrder` out, three more `SortOrder`
  values in VFS-6's sources) but not the index schema.
- **Links**: [VFS-1](../vfs-1-core-search/spec.md) AC-9, AC-10, S6;
  [VFS-6](../vfs-6-bookmarks/spec.md) "Sorting" (extended here, not replaced);
  [VFS-11](../vfs-11-golden-relevance/spec.md) (golden set, must stay green);
  `docs/research/prior-art-proposals.md` P2; GitHub issues 66 and 82.

## Problem

"La última factura de la luz" or "el vídeo más grande" is an order, not a word. Today a file
search has one order, relevance, cut at 2,000 hits. The «Orden» select appears only while
Firefox bookmarks are on, and its «Lo más reciente primero» re-sorts the 2,000 most relevant
hits, so on a broad query it can miss the newest file entirely. A filter-only query lists
newest first, but only among the first 2,000 rows its scan read.

## Acceptance scenarios

Tests cite these ids (`VFS-12 AC-<m>`).

- AC-1 Given a query and the order newest, oldest, name or size When it is searched Then
  every result is in that order: newest or oldest modification first, an unknown date (0)
  last in both; folded name A to Z; largest first with folders last. Ties keep the relevance
  order, and paging returns the same sequence as one big page.
- AC-2 Given more matches than the 2,000-hit cap but at most 6,000 (the sorted read cap)
  When a non-relevance order is chosen Then the list holds the first 2,000 of all matches in
  that order: the newest file is first even when 2,000 better-ranked files exist. The order
  is exact, so `approximateOrder` is false.
- AC-3 Given more matches than the sorted read cap, or a read cut by the deadline, under a
  non-relevance order or a filter-only query Then the response sets `approximateOrder`, and
  the results line says «Más de N resultados: hay demasiados para ordenarlos todos. Escriba
  algo más para afinar.». Relevance never sets it, except on a filter-only query read past
  the 2,000-hit cap.
- AC-4 Given the default order (relevance) Then results and caps are unchanged and the
  VFS-11 golden set stays green.
- AC-5 Given the «Orden» select Then it is on screen whenever there are results, in the
  results header beside the count (VFS-38 R9), with «Lo más parecido primero»
  (default), «Lo más reciente primero», «Lo más antiguo primero», «Por nombre (A-Z)», «Lo más
  grande primero», plus «Archivos primero, luego marcadores» while bookmarks are included.
  With bookmarks, dates are the date saved, name sorts by title, and bookmarks (no size) come
  after every file by size. Each order pages consistently.

## Non-goals

- An index on `mtime` or `size`: that would be a schema change. The read cap and the
  deadline bound the cost instead.
- Clicking column headers, and an ascending/descending toggle per order.
- Remembering the order between sessions: it resets to relevance at start (issue 82).
- Publication dates, NTFS creation dates and EXIF dates (issue 82 "Fuera de este paso").
- Per-row source labels («Archivo», «Marcador»), which issue 82 suggests. The owner chose to
  keep "by source" as an order.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Which orders does the select offer (P2 asked for relevance/newest/name/size; issue 82 for relevance/newest/oldest)? | All five: «Lo más parecido primero», «Lo más reciente primero», «Lo más antiguo primero», «Por nombre (A-Z)», «Lo más grande primero» | owner | 2026-11-20 |
| 2 | Keep «Archivos primero, luego marcadores» as an order, or replace it with per-row labels? | Keep it as an order while bookmarks are included | owner | 2026-11-20 |
| 3 | How to sort before the cap without a schema change | Read up to 6,000 verified candidates (3× the cap), rank them, re-sort stably, then cut to 2,000. Past that, or on the deadline, flag the order as approximate. 20,000 was measured at 200–470 ms warm p95 on 2 M files; with 6,000 every order passed under 150 ms in runs without load bursts from other work (PR #137) | session, after `vfs bench --sort` | 2026-11-21 |
| 4 | Filter-only queries | Under relevance they keep the 2,000-hit read (newest first among it) and flag the order approximate past it: reading 20,000 took the filter-only class from 15 to 185 ms warm p95 | session, after `vfs bench --sort` | 2026-11-21 |
| 5 | Where the order travels | The core gets `SearchQuery.sort`; the screen keeps sending `search_sources`' `options.sort`, which maps onto it (`SortOrder::files`) | session | 2026-11-20 |
