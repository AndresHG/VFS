# What VFS should copy from existing tools, ranked

Companion to [`prior-art.md`](prior-art.md), which holds the evidence and sources for every
reference below. Each proposal says what the reference tool does, why it would help the target user
on the reference machine (an older, non-technical user; 5 M files on one busy Windows HDD shared with
Synology sync; also macOS), what it would cost, what could go wrong, and which phase it belongs to.

Costs are rough engineering effort for one developer with an agent: **S** (≤ 1 day), **M** (2–5
days), **L** (more than a week or a schema change). Phases are the GitHub milestones: Fase 1
(name and path search, in PR 61), Fase 2 (validation on the real PC), Fase 3 (document content),
Fase 4 (polish and distribution), Fase 5 (optional AI assistant). Proposals that already have an
issue point to it instead of opening a new one.

The ranking weighs, in order: how much the user's daily search gets better or more trustworthy,
how much load it removes from the reference machine, then cost.

## Ranked proposals

### P1. Resume from a change cursor instead of re-walking at every start

- **Reference.** Everything replays the NTFS USN journal from its last position at start; Cardinal
  stores the FSEvents id taken before its first walk and resumes the stream from it at every
  launch. Neither re-walks the tree.
- **For the target user.** VFS re-walks every root at every start (spec I4). On 5 M files on a busy HDD
  that is hours of seeking at each launch **[est]**, competing with Synology, with "Comprobando
  cambios…" on screen most of the time. A cursor makes a launch cost seconds.
- **How.** Windows: the unprivileged USN read with a file-reference map (R5, issue 32), and the
  leaf-directory mtime skip as the fallback (R9, issue 34). macOS: open an FSEvents stream with
  `sinceWhen` = the last processed event id (stored in `meta` after each commit); the `notify`
  crate cannot do this, so the macOS watcher needs its own FSEvents loop, as the Windows one
  already has. Full re-walk only when the journal or event history no longer reaches back
  (`ERROR_JOURNAL_ENTRY_DELETED`, a changed journal id, `MustScanSubDirs`, a history-done gap).
- **Cost.** Windows L (issue 32), M (issue 34); macOS M. **Risk.** The USN call is undocumented
  (fallback exists); a missed event between the saved id and the commit must be covered by
  saving the id inside the same transaction as the rows it covers.
- **Phase.** Fase 2, before the first install on the real PC (issue 17). macOS: issue 65.

### P2. Sort control: relevance, newest, name, size

- **Reference.** Every Everything-class tool sorts any result set by any column instantly
  (Everything's fast sort, FSearch's pre-sorted arrays); WizFile and Agent Ransack likewise.
- **For the target user.** "La última factura de la luz" or "el vídeo más grande" is a sort, not a word.
  Today the only order is relevance, capped at 2,000 hits; filter-only queries already list newest
  first.
- **How.** A small "Ordenar: Más relevantes · Más recientes · Nombre · Tamaño" select. For any
  order other than relevance the query must sort in SQL before the cap (`ORDER BY mtime DESC
  LIMIT 2000` over the verified candidates), never re-sort the capped relevance list. With FTS
  candidates that can mean reading all candidates of a broad query; bound it with the existing
  deadline and say "orden aproximado" when cut.
- **Cost.** M. **Risk.** Broad queries sorted by date are slower than relevance; an `mtime` index
  on `files` adds disk. **Phase.** Fase 4.

### P3. Recent searches, one click away

- **Reference.** Everything's search history and "home search"; HoudahSpot's "Recent files"
  template; browser address bars.
- **For the target user.** Older users repeat the same few searches and may not remember the exact
  words that worked last time. A list of the last ten searches under the empty box turns a
  remembered success into one click.
- **How.** Store the last N (query text + filters) in the index's `meta` table or a small settings
  file, shown when the box is empty and focused; a "Borrar historial" link. Nothing leaves the PC.
- **Cost.** S. **Risk.** Privacy on a shared PC; keep it clearable. **Phase.**
  Fase 4.

### P4. Favourite folders as one-click scopes

- **Reference.** Everything's bookmarks (search + filter + sort), Agent Ransack's saved `.srf`
  searches and Favorites, Finder's sidebar.
- **For the target user.** The archive has a few big areas ("Fotos", "Papeles de casa", "Gestoría").
  Choosing one as a scope today means the folder filter; a row of named chips ("Solo en Fotos")
  is easier than a picker.
- **How.** Pin a folder from a result's "mostrar en carpeta" row or from the folder filter; pins
  appear as chips that set the folder filter. Later, "guardar esta búsqueda" stores text + filters
  under a name.
- **Cost.** S–M. **Risk.** Pins to folders that later vanish: show them greyed, never delete
  silently. **Phase.** Fase 4.

### P5. Learn from what the user reveals (frecency)

- **Reference.** Kensaku adds a bounded bonus `0.08·ln(1+opens)·e^(−days/14)` **[doc]**
  (`search/mod.rs:1682-1725` at commit b8b009f); Listary ranks fuzzy
  matches "by your habits"; Windows Search and Spotlight mix usage into ranking.
- **For the target user.** People look for the same documents again (the DNI scan, the deeds). A file
  revealed last week should come first next time, among otherwise equal hits.
- **How.** Count `reveal` per file id (and last time) in a small table; add it as a tie-breaker
  inside a tier (never across tiers, so it cannot bury an exact match). VFS never opens files, so
  "mostrar en carpeta" is the only signal and it is a strong one.
- **Cost.** S–M. **Risk.** Feedback loops (a wrong file keeps winning): keep it within a tier and
  decaying. **Phase.** Fase 4.

### P6. Golden relevance tests in Spanish

- **Reference.** Kensaku's `tests/golden_set.rs`: a fixture tree, 43 queries, Recall@5 and MRR
  asserted in CI.
- **For the target user.** Ranking changes (P2, P5, VFS-4's schema, typo tiers) can silently move the
  right file off the first screen. A fixed set of Spanish queries — accents, ñ, typos, words split
  between folder and name, "2019", short words — catches that before release.
- **How.** A synthetic tree modelled on the real archive's shapes; 40–60 queries with the expected
  file in the top 3; tests cite the AC they prove.
- **Cost.** S–M. **Risk.** The set drifts from real use; refresh it from the owner's real searches
  after Fase 2. **Phase.** Fase 2 (before ranking work).

### P7. "Carpetas que no se pudieron revisar": a visible list

- **Reference.** Recoll's `--diagsfile` writes one line per skipped file in fixed categories, yet
  users report not finding it; Kensaku's health warnings were confusing because they were not
  actionable.
- **For the target user.** When a folder cannot be listed (permissions, a sync lock, a cloud-only
  folder), VFS keeps retrying (I11) but the user only sees missing results. One line in the
  folders panel — "3 carpetas no se pudieron revisar · Ver" — with the paths and a plain reason
  makes a missing file explainable to the owner over the phone.
- **How.** The indexer already counts failures per directory; persist the last error kind per
  directory and list them. Fixed Spanish categories: sin permiso, no disponible, solo en la nube,
  error de disco. Reuse the same list for extraction failures in Fase 3.
- **Cost.** S. **Risk.** Noise from transient locks: list only directories that failed after the
  retries. **Phase.** Fase 2.

### P8. A light, optional query syntax that never gets in the way

- **Reference.** Everything (`!palabra`, quotes, `ext:`), Recoll (`-palabra`, `ext:`, `dir:`),
  Kensaku (unknown `key:` is plain text; parse errors become hints, never zero results).
- **For the target user.** The user will not type operators, but the owner (helping by phone or remotely)
  will, and two cases do come up for anyone: excluding a word ("contrato -borrador") and asking
  for one type by extension ("pdf"). A plain query must keep working exactly as today.
- **How.** Only three forms, all optional: `-palabra` excludes, `"dos palabras"` must appear
  together in that order, and a term that is exactly an extension with a dot (`.pdf`) filters by
  extension. Anything else, including unknown `x:` text, stays literal. No regex, no OR, no
  uppercase operators.
- **Cost.** M. **Risk.** A name that genuinely starts with `-` or contains quotes; keep the literal
  match as a fallback tier. **Phase.** Fase 4.

### P9. A suggestion when nothing is found

- **Reference.** Recoll's clickable spelling suggestions; Kensaku's empty-state hints ("Try a
  different query or broaden your filters"); Google's "¿Quiso decir…?".
- **For the target user.** "Sin resultados" is a dead end. VFS already proves cheaply that no one-typo
  variant exists (S8); when a short word (under 6 characters, no typo tolerance) or a filter is
  the cause, the screen can say so: "Ningún resultado con el filtro *Fotos*. ¿Buscar en todo?" or
  "¿Quería decir *boda*?" for a near spelling of a short word that does exist.
- **How.** On zero hits, rerun once without filters (count only) and once with one-edit variants
  of short terms (count only, bounded), and offer the better option as a button.
- **Cost.** M. **Risk.** Extra queries on a cold HDD; run them only on zero hits and within the
  soft deadline. **Phase.** Fase 4.

### P10. Shrink the name index by indexing each distinct name once

- **Reference.** Cardinal keeps one entry per unique basename and maps it to the files that carry
  it; searches scan only distinct names. Everything compresses little but stores names compactly.
- **For this machine.** Archives repeat names (`IMG_0001.JPG` per camera roll, `Thumbs.db`,
  `factura.pdf`, `Scan.pdf`). If, say, 30 % of names are repeats, an FTS table over distinct
  folded names is 30 % smaller and cheaper to read cold from the HDD **[est]**.
- **How.** First measure the distinct-name ratio on the real archive (one SQL count during Fase 2).
  If it is high: `names(id, name_key UNIQUE)`, `files.name_id`, `files_fts` over `names`, a join
  back to `files`. Coordinate with VFS-4, which owns the schema.
- **Cost.** S to measure; L to change the schema. **Risk.** A join per candidate; renames touch
  the name table. **Phase.** Fase 2 (measure), then decide.

### P11. Show copies of the same file together

- **Reference.** Everything's `dupe:` and `sizedupe:`; Kensaku groups versions and screenshot
  bursts.
- **For the target user.** A synced archive with backups holds the same document in several folders.
  Seeing "3 copias" on one row, expandable, answers "¿cuál es la buena?" and shortens the list.
- **How.** Group hits with the same folded name and size in the ranked list (no hashing, no file
  reads); the newest copy leads.
- **Cost.** M. **Risk.** Same name and size but different content is possible; label it "mismo
  nombre y tamaño", never "duplicado". VFS never deletes. **Phase.** Fase 4.

### P12. Fase 3 extraction: copy the failure handling, not the engines

- **Reference.** Recoll (`filtermaxseconds`, `textfilemaxmbs`, `indexallfilenames`), ripgrep-all's
  cache keyed by extractor version and validated by mtime, Kensaku's per-file timeouts,
  `catch_unwind`, bounded channel and pending/retry flags, DocFetcher's out-of-memory crashes as
  the counter-example.
- **For the target user.** Content search will read terabytes from the busy HDD; one hostile PDF must not
  crash or stall it, and an extractor update must not re-read everything.
- **How.** Already planned in issue 23 (isolated process); add: a timeout of 10–30 s per file (not
  Recoll's 1,200 s), a text cap per file, an `extractor_version` per format in the content state
  so only that format is redone when it changes, validity by mtime **and** size (rga checks only
  mtime), retries only when the extractor version changes, names always indexed whatever happens
  to the content, and failures in the P7 list.
- **Cost.** Part of issues 23–24. **Risk.** None beyond Fase 3's own. **Phase.** Fase 3 (comment
  on issue 23).

### P13. Static embeddings as the cheapest semantic option

- **Reference.** Kensaku's default embedder is potion-base-8M (MIT): a lookup table stored as i8, about 7.3 MiB
  **[doc]** (`embed/table.rs` at commit b8b009f), a
  weighted mean of token vectors, no ONNX runtime at query time, exact scan over i8 vectors.
- **For this machine.** If semantic search is ever worth it (issue 41), this is the only model
  class that a slow PC can run over a big archive without a GPU.
- **Cost.** Part of issue 41. **Phase.** Fase 5 (comment on issue 41).

### P14. Copy Kensaku's cloud-placeholder test

- **Reference.** Kensaku indexes files with `FILE_ATTRIBUTE_OFFLINE` or
  `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` by name only and does not list cloud-only folders
  (`walker.rs:85-110,188-210`, MIT).
- **For this machine.** Synology Drive and OneDrive can leave placeholders; listing or opening them
  can trigger downloads. Issue 49 already covers the policy; this adds a reference implementation.
- **Cost.** S. **Phase.** Fase 2 (comment on issue 49).

### P15. Plain-Spanish status when indexing backs off

- **Reference.** Windows Search backs off silently ("waiting for computer to become idle") and its
  index stays incomplete with no visible sign — its top complaint.
- **For the target user.** When the planned busy-disk back-off (R3 in `windows-recommendations.md`, issue 20)
  pauses the walk, results may be incomplete.
  The status line should say so in words ("Revisión en pausa mientras el disco está ocupado; puede
  faltar algún archivo reciente"), not just stop moving.
- **Cost.** S. **Phase.** Fase 2 (comment on issue 20).

## Things VFS should deliberately not do

| What | Who does it | Why not here |
|---|---|---|
| An elevated service or MFT reader for speed | Everything, AnyTXT, WizFile, UltraSearch | Needs an admin install; a raw-volume reader is what antivirus flags; Everything's service hands every local user the full list of file names. The no-admin rule is a product decision. |
| Use Everything as an optional backend through its SDK | Flow Launcher, PowerToys plugins | Makes the fast path depend on a separately installed, admin-installed, Windows-only program that must be running in the same session; two indexes to explain to the user. |
| Delegate to Windows Search or Spotlight | launchers, HoudahSpot | Windows Search documents a 1 M-item ceiling and backs off silently on a busy disk; Spotlight misses files with no explanation and needs rebuilds. |
| A RAM-resident index loaded at every start | Everything, FSearch, Cardinal, Kensaku | ~0.5 GB at 5 M files breaks the 300 MB budget on a small-memory PC, and a snapshot saved on exit loses the session after a crash (Everything 1.4). Kept as the decision log's fallback if FTS5 is too slow on the real HDD. |
| Regex, uppercase boolean operators, `case:`, `soundex:` in the search box | Everything, Agent Ransack, Recoll | A wrong guess at syntax returns nothing; the target user should never meet a mode. P8 keeps only three forgiving forms. |
| Accent-strict search when the query has an accent | Recoll (`autodiacsens`), Everything's Audio/Video filters | Users type accents inconsistently; one rule everywhere is easier to trust. |
| Smart case | fd | Case never matters to this user; capitals typed by habit must not change results. |
| A dense column grid with headers and a menu bar | Everything, WizFile | Hostile to an older reader (see `docs/design/references.md`); P2 gives sorting without a grid. |
| Previews, thumbnails, opening files | Everything 1.5, Kensaku, DocFetcher | Out of scope by decision 7; previews read file contents and wake antivirus on the HDD. |
| Content index the size of the content, or content held in RAM | Recoll, Everything 1.5 | 4–5 TB of documents; Fase 3 must cap text per file and keep it on disk. |
| A second search engine (Tantivy) beside SQLite | Kensaku | Two stores to commit in order and repair after a crash; FTS5 already does BM25 for Fase 3. |
| A health "doctor" with technical warnings | Kensaku | Its users could not act on the warnings (issues 94, 142, 146, 176). P7 lists facts with a plain reason instead. |
| Duplicate deletion | Everything (`dupe:` + delete), cleanup tools | VFS never writes to user files; P11 only groups copies. |
| One walker thread per core | fd's default | On one HDD more threads mean more seeks; fd's maintainers doubt gains past a few threads. |
| An HTTP server, plugins, a remote index server | Everything 1.5, AnyTXT | Attack surface and settings with no benefit for one local user. |
| Semantic search as the answer to typos | Copilot+ Windows Search | Needs an NPU, covers only indexed places, and does not fix the real failures (a missing accent, a slip, remembering the folder instead of the name), which VFS already handles. |

## Issues

Created in `AndresHG/VFS` for this study (titles in Spanish, as the tracker is):

| Proposal | Issue | Milestone |
|---|---|---|
| P1 Windows | existing 32, 34 | Fase 2 |
| P1 macOS | 65 (reanudar FSEvents desde el último evento) | Fase 2 |
| P2 | 66 (ordenar resultados) | Fase 4 |
| P3 | 67 (búsquedas recientes) | Fase 4 |
| P4 | 68 (carpetas favoritas) | Fase 4 |
| P5 | 69 (aprender de "mostrar en carpeta") | Fase 4 |
| P6 | 70 (pruebas de relevancia en español) | Fase 2 |
| P7 | 71 (lista de carpetas que no se pudieron revisar) | Fase 2 |
| P8 | 72 (sintaxis mínima opcional) | Fase 4 |
| P9 | 73 (sugerencia cuando no hay resultados) | Fase 4 |
| P10 | 74 (medir nombres repetidos) | Fase 2 |
| P11 | 75 (agrupar copias) | Fase 4 |
| P12 | comment on existing 23 | Fase 3 |
| P13 | comment on existing 41 | Fase 5 |
| P14 | comment on existing 49 | Fase 2 |
| P15 | comment on existing 20 | Fase 2 |

Issues 32 and 34 also got a comment recommending them before the first install on the real PC
(issue 17).
