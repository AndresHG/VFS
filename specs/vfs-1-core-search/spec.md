# VFS-1 — Core search: local file-name and path search for a huge personal archive

- **Status**: approved
- **Size**: L
- **Links**: converted from the approved deep-interview spec of 2026-11-20 (interview held in Spanish)

## Problem

A non-technical user keeps a 4–5 TB personal archive on fixed internal HDDs (designed for 5
million files) on a slow Windows PC, and cannot find files in it. VFS is an owned, extensible
desktop app for Windows and macOS that finds any file instantly by file name and folder path,
with filters, typo and accent tolerance, and a link that opens Explorer/Finder at the result.
It is meant for anyone: tuned to run well even on that slow PC, and just as well on ordinary
or fast machines. Searching inside text documents is a later optional extension. Target for today: the app
working on the owner's Mac plus a Windows installer built by CI.

## Constraints

- Stack: Rust core, SQLite (WAL) with FTS5 for names and paths, Tauri 2 app. Windows 10 or
  later; no administrator rights.
- Data: fixed internal HDDs (maybe SSD later). Only the folder(s) the user selects on a local
  disk are indexed. The backup NAS is not indexed.
- The reference machine is slow: indexing runs in the background, at low priority, resumable; metadata
  comes from directory enumeration without opening files.
- Fully offline. No LLM in v1.
- Simple UI with options, in Spanish.
- Reuse existing work where possible (Everything, Kensaku (MIT), etc.), respecting licences.
- Private GitHub repository `VFS`; GitHub Actions with a Windows + macOS matrix; unsigned
  installer (a small, known audience; the SmartScreen warning is accepted).
- Engineering system inherited from Calligence and adapted: spec-first, tests cite `AC-n`,
  fresh-context review (pr-critic + rubric), git guard hooks, CLAUDE.md budget.

## Requirements (EARS)

- R1. THE repository SHALL carry the adapted engineering system (rules, rubric, critic, spec
  skill, hooks) and a CI that lints and tests on Windows and macOS.
- R2. WHEN the user adds or removes a root folder, THE indexer SHALL index exactly the files
  under the selected root folders.
- R3. WHILE indexing runs, THE app SHALL show progress and SHALL answer queries over what is
  already indexed.
- R4. IF indexing is interrupted, THEN THE indexer SHALL resume without starting over.
- R5. WHEN a file is created, deleted, renamed or moved under a root folder, THE index SHALL
  eventually reflect it (watcher plus periodic rescan).
- R6. THE indexer SHALL index and find paths longer than 260 characters and Unicode names in
  NFC or NFD.
- R7. WHEN the user types a query, THE search engine SHALL match file name and any segment of
  the path, case- and accent-insensitively, tolerating one typo, within the latency and memory
  budget of AC-9.
- R8. THE search engine SHALL combine filters by type, date, size and folder.
- R9. THE app SHALL be a single Spanish-language screen with a search box, results as the user
  types, and visible filters.
- R10. WHEN the user chooses "mostrar en carpeta" on a result, THE app SHALL open
  Finder/Explorer with that file selected.
- R11. THE CI SHALL produce a downloadable Windows installer and a macOS build.
- R12. THE codebase SHALL contain a written design and an extension point for document
  content search, without implementing it in v1.

## Acceptance scenarios

Tests cite these ids (`VFS-1 AC-<m>`) in their doc comments.

- AC-1 (R1) Given the project folder, When setup is complete, Then a git repository exists, a
  private GitHub repository is created and the code is pushed to it.
- AC-2 (R1) Given the repository, When a session starts, Then `CLAUDE.md`, the review rubric,
  the pr-critic agent, the `spec` skill and the adapted hooks are present and wired in
  `.claude/settings.json`.
- AC-3 (R1) Given a push or PR, When GitHub Actions runs, Then lint and tests run on Windows
  and macOS and pass.
- AC-4 (R2) Given one or more root folders, When the user adds or removes one, Then only what
  hangs below the selected root folders is indexed.
- AC-5 (R3) Given indexing in progress, When the user searches, Then progress is visible and
  results come from what is already indexed.
- AC-6 (R4) Given an interrupted indexing run, When the app starts again, Then indexing
  resumes without starting from zero.
- AC-7 (R5) Given an indexed root folder, When a file is created, deleted, renamed or moved
  on disk, Then the index eventually reflects the change (watcher + periodic rescan).
- AC-8 (R6) Given paths over 260 characters and names in NFC and NFD, When indexed, Then they
  are found.
- AC-9 (R7) Given a synthetic tree of 2 M files on the Mac, When queries run, Then each
  returns results in < 150 ms (p95) and the app uses < 300 MB of RAM.
- AC-10 (R7) Given indexed files, When the user searches, Then the query matches the name and
  any segment of the path, ignores case and accents ("notaria" finds "Notaría") and tolerates
  one typo.
- AC-11 (R8) Given indexed files, When the user combines filters by type (document, photo,
  video, audio, other, folder), date, size and folder, Then only files matching all of them are shown.
- AC-12 (R9) Given the app is open, When the user looks at it, Then there is one screen with a
  search box, results as they type, visible filters and Spanish copy.
- AC-13 (R10) Given a result, When the user chooses "mostrar en carpeta", Then Finder/Explorer
  opens with the file selected.
- AC-14 (R11) Given a CI run, When it finishes, Then a downloadable Windows installer and a
  macOS build exist, and the app starts and searches on the Mac.
- AC-15 (R12) Given the codebase, When reviewed, Then a written design and an extension point
  exist for document content search (extraction + Spanish BM25; optional embeddings later),
  not implemented today.

## Non-goals

- Searching by the content of images, audio or video (found by name, path and filters only).
- Opening files from the app, and previews.
- Indexing a NAS, network drives or removable external disks (v1).
- LLM, reranker, OCR (v1).
- Code signing, auto-update, Microsoft Store (v1).
- Reading the MFT/USN journal directly with administrator rights (v1).

## Pre-mortem

- HDD scan time: unmeasured on the real hardware. Flat walk with one I/O stream per disk, low
  priority, resumable (AC-5, AC-6); metadata from enumeration only.
- Long paths and case: covered by AC-8; Windows needs long-path handling end to end.
- Unicode normalisation: names are stored and folded so NFC and NFD match (AC-8, AC-10).
- Antivirus: enumeration does not open files; the effect of Defender on the index database
  is unmeasured — a known risk.
- Index corruption: SQLite WAL with transactions; a corrupt index is rebuilt from a rescan
  (AC-6, AC-7).
- Huge result sets: results are capped or paged so AC-9 holds for broad queries.
- Permission-denied, symlinks, junctions, cloud placeholders: skipped or not followed, so a
  walk never loops; the exact policy is an implementation decision recorded in the PR.
- Writes to user files: none. v1 reads metadata and asks the OS to reveal a file (AC-13).

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Which components? | Engineering system, indexer, content understanding (reduced), search engine, desktop app, packaging | owner | 2026-11-20 |
| 2 | What does the user search for? | Name or folder; document content optional later; media out of search | owner | 2026-11-20 |
| 3 | Where does the data live? | Fixed internal HDDs, selected folders; backup NAS not indexed | owner | 2026-11-20 |
| 4 | What must exist by 18:00 today? | App on the Mac + Windows installer from CI | owner | 2026-11-20 |
| 5 | Why not Everything? | Untried; an owned, extensible product optimised for huge archives, copying what works | owner | 2026-11-20 |
| 6 | Stack? | Rust + SQLite + Tauri 2 | owner | 2026-11-20 |
| 7 | Minimum for v1? | Typo and accent tolerance, filters; no opening files, only "mostrar en carpeta" | owner | 2026-11-20 |
| 8 | Repository and CI? | Private repo on the owner's account, GitHub Actions, unsigned installer | owner | 2026-11-20 |
| 9 | Scale and performance targets? | Designed for 5 M files; synthetic test of 2 M, < 150 ms p95, < 300 MB | owner | 2026-11-20 |
| 10 | Ticket system? | GitHub Issues in `AndresHG/VFS`, one milestone per phase (Fase 0 Cimientos, Fase 1 Búsqueda por nombre y ruta, Fase 2 Validación en el PC real, Fase 3 Contenido de documentos, Fase 4 Pulido y distribución); spec ids stay `VFS-n` in `specs/` | owner | 2026-11-20 |
| 11 | Branch model? | Protected `main` + `feat/<title>` branches, squash merge (default) | owner | 2026-11-20 |
| 12 | Hook runtime? | Python 3 and bash, on the macOS dev machine only (default) | owner | 2026-11-20 |
| 13 | Are folders results? | Yes. A folder whose name matches is its own result row (empty folders included), marked as a folder (`isDir` on the hit), with "mostrar en carpeta" revealing the folder; the type filter gains "carpeta" (`folder` kind) | owner | 2026-11-20 |
| 14 | WebView2 on Windows? | `embedBootstrapper` (small installer): the reference machine has internet at install time | owner | 2026-11-20 |
| 15 | May the live watcher be cut for time? | No; it is the last item in any cut list | owner | 2026-11-20 |
| 16 | UI wording? | Impersonal (usted) Spanish in plain words; user-facing text avoids "indexar", "índice", "reindexar" ("preparar la búsqueda", "revisar carpetas", "volver a revisar") | owner | 2026-11-20 |

## Tauri commands and events

The desktop app talks to the core only through these commands (Rust names; every argument
is one word, so JS passes the same keys, except `isDir`, which Tauri maps to `is_dir`).
Every command runs off the main thread and rejects with `ApiError { code, message }`, where
`code` is one of `not_found`, `not_a_directory`, `overlapping_root`, `unavailable`,
`schema_too_new`, `db`, `io`. Payload types are the camelCase serde types of
`crates/vfs-core/src/types.rs`; `ApiError`, `RevealOutcome` and the event names are in
`crates/vfs-core/src/api.rs`.

| Command | Arguments | Returns |
|---|---|---|
| `list_roots` | — | `Root[]` |
| `pick_folder` | — (native folder dialog) | `string \| null` |
| `add_root` | `path: string` | `Root` |
| `remove_root` | `id: number` | `null` |
| `search` | `query: SearchQuery` | `SearchResponse` (`hits[].isDir`, `truncatedBy: null \| "cap" \| "deadline"`) |
| `reveal` | `id: number`, `isDir: boolean` | `RevealOutcome`: `"revealed"` or `"opened_parent"` |
| `get_progress` | — | `IndexProgress` |
| `rescan` | `id: number \| null` | `null` |
| `set_paused` | `paused: boolean` | `null` |

Events: `index-progress` (payload `IndexProgress`, at most every 500 ms) and `index-changed`
(payload `null`, at most every 2 s, after the indexer commits).

Every `Root` carries `drive` (`"local"`, `"network"`, `"removable"` or `"unknown"`), asked of
the system when roots are read, so the UI warns when a chosen folder is on a network or
removable drive (U4). A root being removed does not block adding the same folder again:
the new root supersedes the removal.

## Implementation decisions

Ids cited by code and tests next to the AC ids. D1–D4 are decisions 13–16 above.

- C4. The search deadline comes from `SearchConfig` (default 1000 ms).
- C5. An index that cannot be opened or fails `quick_check` (run only after an unclean
  shutdown) is renamed to `index.db.bad` and rebuilt from its roots; the `.bad` file and
  its companions are deleted once every root has finished a full scan.
- C6. The app keeps the index in the local app data directory (Tauri `app_local_data_dir`;
  `%LOCALAPPDATA%` on Windows), never the roaming one, so a multi-GB file is not synced or
  backed up with the profile; at start it moves, once, an index an earlier build left in
  the roaming directory.
- S1. For each root, query terms already contained in the root's own path are ignored,
  unless none would remain.
- S2. Candidates are retrieved in ranking-tier order, so a full list never pushes out a
  better tier: names starting with the first term, then names holding every term (read up
  to 4,000 and ranked before the cut to 2,000 hits), then files whose relative path and
  name hold every term (one intersection in `file_paths_fts`, schema version 2), then
  typos. Filters are SQL predicates of every stage. Every candidate is verified against all
  terms.
- S3. The deadline applies to scans; FTS-driven stages run to completion, bounded at about
  2 s.
- S4. Typo tolerance: first the deletions and adjacent transpositions of a term (anchored at
  the start of the name, then anywhere in it), then, for each edit position, FTS candidates
  from the two parts of the term around it (parts shorter than 3 characters dropped); all
  verified within one edit. A typo whose name starts near the first term ranks first.
- S5. Terms shorter than 3 characters cannot use the trigram index: they are verified in
  Rust, or scanned when they are the only terms.
- S6. A new query interrupts the running one; `\` is folded to `/`; filter-only queries list
  newest first; the folder filter is a directory range in SQL.
- S7. The last ranked list is cached by (query, filters, index version), so pages are
  consistent.
- S8. A query with no hit after the path tier first probes `file_paths_fts` and `dirs_fts`
  once for rows holding, for every term, the term or one of its one-edit alternatives (S4);
  when none exists the typo tiers are skipped. The path-index intersection uses the
  trigrams that cover each term without overlap, not all of them.
- S9. At most 1 KB of query text and 128 characters per term are read; terms over 64
  characters get no typo tolerance. The search box accepts at most 1,024 characters.
- I1. A watcher event never downgrades a directory pending a deep re-list.
- I2. Windows watcher: `ReadDirectoryChangesW` with a 64 KB buffer; an overflow or error
  requests a re-walk of the root (I10). macOS: FSEvents through `notify`, honouring its
  rescan flag.
- I3. Watcher events under the app data directory are ignored.
- I4. At start, a root whose change cursor is missing or whose history is lost (VFS-9) is
  owed a catch-up re-walk (resumable, low priority, searchable meanwhile); while running, roots are re-walked when
  their last full scan is 7 days old or stored in the future (a clock set back).
- I8. One exclusion list (`walk::is_excluded`: system names, `.SynologyWorkingDirectory`,
  `@eaDir`, `#recycle`, `#snapshot`, `~$*`, `.~lock.*#`, `~*.tmp`, ...) serves the walker and
  the watcher, which drops events for any excluded component below the root before
  anything is marked dirty.
- I9. A directory rename or move inside a root that the Windows watcher reports as an
  old/new pair is applied as a path update of the subtree's `dirs` rows, folder search
  entries and the files' `file_paths_fts` rows in one transaction; anything else falls
  back to re-listing both parents.
- I10. Full re-walks run one at a time and a walk in progress is never restarted: a request
  for a root being walked owes one more pass after it. Requests from lost events
  (overflow, a watch that failed), and those owed by a root that was being walked, wait
  until that root's last full scan is 1 hour old; start-up and manual requests do not.
- I11. A directory that fails to list keeps its rows, stays pending and counts as an error;
  it is retried after 1 min, doubling, and after 6 failures waits for the next full walk.
- I12. A root that cannot be watched is retried every minute and as soon as it is
  reachable again; once watched it owes a re-walk (I10). Marking a folder dirty from the
  app (a missing file at "mostrar en carpeta") wakes the indexer at once.
- I5. The indexer commits every ~20,000 changed rows or 5 s.
- I6. Roots left in `removing` are finished at start (also by the CLI).
- I7. The watcher tests run on Windows CI too, polling up to 30 s.
- U1. A deadline-cut search with no hits says "La búsqueda no terminó; inténtelo de nuevo.",
  never "Sin resultados", and is not cached.
- U2. During the first scan the UI says the user can search meanwhile and that closing the
  program resumes the scan later.
- U3. Results from unavailable roots are marked, and the banner names the missing folder.
- U4. Adding a folder that overlaps a root names the relation (same, inside, contains);
  a folder on a network or removable drive gets a warning.
- U5. The development fake backend lives outside the bundled frontend and cannot load in
  the app.
- U6. The status bar says "Revisando carpetas…" during the first scan, "Comprobando
  cambios…" during any later scan (the catch-up at start included) and, when idle,
  "Última revisión: hoy 9:14" with the oldest completed check of the reachable folders;
  the manual button reads "Buscar cambios ahora".
- X1. One desktop test drives add_root, search and get_progress over IPC on Tauri's mock
  runtime, on both systems in CI.
- X2. CI also runs the CLAUDE.md budget check and the UI tests (`node --test`: status
  wording, and the screen's copy and markup for AC-12). A Rust test keeps the closed sets
  of `apps/desktop/ui/contract.js` equal to the serde values of the core's enums.
- X3. Workflow actions are pinned by commit SHA, the Rust version by `rust-toolchain.toml`;
  release builds use `--locked`, no build cache, and publish a SHA-256 checksum per
  artifact.
- C7. The app writes warnings and errors to a rotating log file in the local app data
  directory (1 MB, two old files); nothing below warning level.
- I13. Before a directory is re-listed, the root and every folder on its path are checked:
  one replaced by a link since it was indexed is treated as removed, a root that became a
  link as unavailable.

## Technical context

- Planned design: flat directory walk (one I/O stream per HDD), SQLite WAL + FTS5 trigram over
  the accent-folded name, directory paths in their own table, volumes identified by GUID or
  serial. Schema version 2 adds `file_paths_fts`, each file's folded relative path and name
  as trigrams without positions (`detail=none`). Measured on the 2 M-file synthetic index:
  +311 MB (577 MB → 889 MB); the same table with positions was put at about +1 GB during
  the design (not re-measured). Unmeasured risks: HDD scan time and FTS5 trigram at several
  million names (alternative: an in-memory name buffer).
- Document content search (GitHub milestone Fase 3): extraction in an isolated worker,
  Spanish BM25; small embeddings optional. BM25 outperforms dense models on long Spanish
  documents.
- No figure from the research has been measured on the real hardware.

## Open questions

None blocking.
