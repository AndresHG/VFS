# VFS-44 — Sources and notes: one item model, the user's own notes, collections

- **Status**: draft. The owner decided D1–D3 on 2026-11-25; D7 too; D4–D6 are asked and carry
  the recommended choice until answered.
- **Size**: L. VFS writes files for the first time (the user's notes, in a folder VFS owns),
  it adds a third source and a derived store, it reads a platform-specific default folder
  (Documents) on Windows and macOS, and it is multi-PR.
- **Links**:
  - [VFS-6](../vfs-6-bookmarks/spec.md): the second source, `search_sources`, «Origen».
  - [VFS-12](../vfs-12-sort/spec.md): sort orders, «por origen».
  - [VFS-42](../vfs-42-scope-tabs/spec.md): «Nombres» and «Documentos» tabs.
  - [VFS-28](../vfs-28-ask-archive/spec.md): what may leave the PC (D1 here keeps its rule).
  - Research: `docs/research/future-lines-roadmap.md` ("A common source abstraction",
    «Fase 6 · Fuentes y colecciones»), `docs/research/future-lines.md` line 1,
    `docs/vision.md` ("Adding to the knowledge base").
  - Issues #77 (reúne todo sobre un tema), #119 (Obsidian).

## Problem

VFS finds what the user already has, but nothing the user adds. The vision's fourth verb,
«ampliar», has no home: a thought, a list or a link to keep has to go into some other app,
where VFS search does not reach it. The user's most frequent request in the feedback
research is thematic («reúne todo lo que tengo sobre X», four of six examples), and today the
answer is a search that is gone when the window closes. Every later knowledge feature
(entities, compiled pages with citations, a graph, chat importers) needs one item model
across sources and one folder where what VFS and the user write lives. This spec builds
that base, without AI: a common source model, notes as a third source, and collections.

## What we know

- Search over several sources is `vfs_bookmarks::sources::search_sources`: files come from
  the core in 500-hit pages, bookmarks from an in-memory list matched with
  `vfs_core::matching`, merged by (tier, folded length, newer first) or by the VFS-12 order;
  `Source` is `Files | Bookmarks` (`crates/vfs-bookmarks/src/types.rs`), and «Origen» maps
  to it in `apps/desktop/ui/sources.js`.
- A hit is `SourceHit::Files(SearchHit)` or `SourceHit::Bookmarks(BookmarkHit)`; each kind
  has its own action («Mostrar en carpeta», «Abrir marcador»).
- Document text lives in `content.db` (VFS-2, VFS-27) only while content search is on; with
  it off no worker runs (VFS-27 R5).
- VFS never writes outside its data folder today. Bookmarks are read from a copy of
  Firefox's files (VFS-6 "Reading").
- Assumptions: a typical user writes tens to a few thousand notes, each a few KB; nobody
  needs more than 10,000 notes. Notes are plain text, so reading them needs no extraction
  worker.

## Requirements (EARS)

Notes folder

- R1. THE app SHALL keep the user's notes as Markdown files (`.md`, UTF-8) in one notes
  folder: by default `«Documentos»/VFS Notas` (Windows `%USERPROFILE%\Documents`, macOS
  `~/Documents`), created on the first note, and changeable in Ajustes (D4). Changing it
  SHALL NOT move or delete any file; VFS reads the new folder from then on.
- R2. THE app SHALL write, rename and delete files only inside the notes folder, and only
  `.md` files it created or that the user created there. A delete SHALL go to the system's
  trash (Papelera), never a permanent delete.
- R3. IF the notes folder is inside a root the user indexes, THEN THE app SHALL show its
  notes once, as notes, and not also as files.

Writing notes

- R4. WHEN the user chooses «Nueva nota», THE app SHALL open a note panel with a title and
  a plain-text body (D5), and SHALL save it to `<title>.md` after 1 s without typing and on
  closing the panel. The file name is the title with characters that no system allows
  replaced, made unique with « (2)», « (3)».
- R5. Each note file SHALL start with YAML front matter holding `id` (a UUID that never
  changes), `created` and `updated` (ISO 8601, local time with offset) and, when set, `tags`.
  A file without front matter is still a note; VFS adds front matter only when it saves that
  note.
- R6. IF a note file changed on disk after the panel opened it (another editor, a sync
  client), THEN THE app SHALL NOT overwrite it: it SHALL say «Esta nota ha cambiado fuera de
  VFS» and offer «Cargar la versión del disco» and «Guardar como nota nueva».
- R7. THE note panel SHALL offer «Abrir en tu editor», which opens the file with the
  system's default app for `.md`.
- R8. FROM any result (file, bookmark or note), THE app SHALL offer «Añadir a una nota»,
  which appends a Markdown link to the item (a `file://` URL for a file, its address for a
  bookmark, a relative link for a note) and its name to a note the user picks or a new one.

Notes as a source

- R9. THE app SHALL search notes like the other sources: the title in the name tiers
  (VFS-1), the title and body under «Documentos» (VFS-42), with the same accent folding,
  typo rule and filters (date = `updated`; size and type filters leave notes out). This
  SHALL work whether or not content search is on.
- R10. «Origen» SHALL offer «Notas» beside «Archivos» and «Marcadores», and «por origen»
  (VFS-12) SHALL order files, then bookmarks, then notes. A note row SHALL show a note icon,
  its title, its first line and its updated date; its action is «Abrir nota».
- R11. WHEN a file in the notes folder is added, changed, renamed or removed by any program,
  THE app SHALL reflect it in search within 5 s while the app is open, and at the next start
  otherwise.

Collections

- R12. THE app SHALL let the user save the current search, with its words, sources and
  filters, as a collection (D6) named by the user. A collection SHALL be a `.md` file in
  `Colecciones/` inside the notes folder, with the search, the pinned items and the excluded
  items in its front matter and the user's description as its body.
- R13. WHEN the user opens a collection, THE app SHALL show its pinned items first, in the
  order pinned, then the current results of its search without the excluded items. Each
  row SHALL offer «Fijar» or «Quitar de la colección».
- R14. IF a pinned item no longer exists at its locator, THEN THE app SHALL show it as «No
  encontrado» with «Buscar por nombre», and SHALL NOT drop it from the file without the
  user's action.

Common source model

- R15. Every hit from every source SHALL carry `source` (`files`, `bookmarks`, `notes`),
  `kind`, a `locator` (an absolute path, an address, or a note's path) and a `title`, so
  one result list, one sort, collections and later importers handle every source the same
  way. Importers (chat and YouTube exports) are not part of this spec (D7); they will add a
  `source` value each.

Footprint

- R16. With no notes folder, notes SHALL add no file, thread, watcher or memory. With
  10,000 notes of 4 KB, notes SHALL add at most 20 MB of resident memory, search p95 SHALL
  stay within VFS-1's budget, and start-up SHALL not wait for the notes to be read (VFS-39).
- R17. Notes and collections SHALL never be sent anywhere. Only VFS-28's rule applies: when
  the user asks a question, the passages used may leave the PC (D1).

## Acceptance scenarios

Tests cite these ids (`VFS-44 AC-<m>`) in their doc comments.

- AC-1 (R1, R4, R5) Given no notes folder, When the user creates a note «Receta de la abuela»
  and types for 2 s, Then `Documentos/VFS Notas/Receta de la abuela.md` exists with front
  matter `id`, `created`, `updated` and the typed body.
- AC-2 (R2) Given a notes folder and a root folder, When every note operation runs (create,
  rename by retitling, delete), Then no file outside the notes folder changes, and the
  deleted note is in the system trash.
- AC-3 (R6) Given a note open in the panel, When another program rewrites the file and the
  user keeps typing, Then the disk file keeps the other program's text and the panel shows
  «Esta nota ha cambiado fuera de VFS».
- AC-4 (R9, R10) Given content search off and a note whose body says «fianza del piso», When
  the user searches «fianza» on «Documentos», Then the note is a hit with source `notes`,
  and «Origen: Archivos» hides it.
- AC-5 (R3) Given the notes folder inside an indexed root, When the user searches a note's
  title, Then the note appears once, as a note.
- AC-6 (R11) Given the app open, When a note file is created by another program in the
  notes folder, Then a search finds it within 5 s.
- AC-7 (R8) Given a file hit, When the user chooses «Añadir a una nota» and picks a note,
  Then the note ends with a Markdown link whose target opens that file.
- AC-8 (R12, R13) Given a saved collection with one pinned item and one excluded item, When
  the user opens it after a new matching file is indexed, Then the pinned item is first, the
  new file is listed, and the excluded item is not.
- AC-9 (R14) Given a collection pinning a file that was then deleted, When the user opens it,
  Then the row reads «No encontrado» and the collection file still lists it.
- AC-10 (R15) Given files, bookmarks and notes that match one query, When it is sorted «por
  origen», Then the list is files, then bookmarks, then notes, and every hit carries
  `source`, `kind`, `locator` and `title`.
- AC-11 (R16) Given no notes folder, When the app runs a search session, Then no notes
  thread, watcher or file exists; and Given 10,000 generated notes, Then the notes store adds
  at most 20 MB RSS and the window shows before the notes are read.

## Non-goals

- Rich text, images or attachments in notes; a Markdown preview.
- Reading an Obsidian vault or other Markdown folders as notes (they are already searchable
  as files); writing anywhere but the notes folder (#119 stays research).
- Chat, YouTube or other export importers (D7); browsers other than Firefox (#197).
- Entities, compiled pages (fichas), «Relacionados», the graph and any AI step: later
  specs on top of this one (see the exploration of 2026-11-25 summarised in D1–D3).
- Sync between computers. The notes folder is a plain folder; a sync client can carry it.
- Sharing or exporting a collection beyond the `.md` file itself.

## Pre-mortem

- Slow PC and HDD: notes are few small files in one folder; reading them is one directory
  listing and at most 10,000 small reads at start, in the background, low priority (R16).
  Writes are one small file per save.
- Long paths: the notes folder sits under Documents; titles are capped at 120 characters so
  the path stays under 260 unless the user chose a deep folder, where `\\?\` paths apply as
  in VFS-24.
- NFC/NFD: titles are stored as typed (NFC) and folded with `fold_key` like file names;
  macOS may return NFD names from the listing, so comparisons go through the fold.
- Antivirus and sync clients: Defender or OneDrive may lock or rewrite a note; R6 prevents a
  silent overwrite, and a save that fails is retried and then reported, never dropped.
- Crash mid-write: a save writes `<name>.md.tmp` beside the note and renames it over the
  note, so a note is never half written; stray `.tmp` files are removed at start.
- A million matches: notes are capped by the R16 assumption; collections reuse the paged
  search.
- Permission-denied, links, placeholders: if the notes folder cannot be written, the panel
  says so and keeps the text in memory; a cloud-only note (OneDrive) is listed by name and
  opened only when the user opens it.
- Opening, writing, moving or deleting user files: yes, for the first time, and only inside
  the notes folder (R2, AC-2). This is the main risk of the spec.
- Does any of this already ship? `grep -rn "notes\|notas"` over `apps/desktop/ui/*.js` and the
  crates finds no notes or collection feature; «Origen» and `Source` are VFS-6's two values.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | What may leave the PC to build knowledge? | Only what the user touches: the passages of a question or a compiled page, as in VFS-28; no background processing by API | owner | 2026-11-25 |
| D2 | Where do compiled pages (fichas) live? | Markdown in VFS's notes folder (`fichas/`), one per entity or topic | owner | 2026-11-25 |
| D3 | What goes first after 0.4? | Sources and notes (this spec), before entities and AI pages | owner | 2026-11-25 |
| D4 | Where is the notes folder? | Visible, `Documentos/VFS Notas`, changeable (recommended; asked) | owner | pending |
| D5 | How are notes written? | A plain-text editor in VFS plus «Abrir en tu editor» (recommended; asked) | owner | pending |
| D6 | What is a collection? | A saved search plus pinned and excluded items, stored as `.md` (recommended; asked) | owner | pending |
| D7 | Do export importers come now? | No, one spec each later | owner | 2026-11-25 |
| D8 | How are notes searched with content search off? | A small derived `notes.db` (FTS5 over folded title and body, same analysis as `content.db`) rebuilt from the folder; the `.md` files are the truth | agent | 2026-11-25 |

## Open questions

- D4–D6 wait for the owner; each changes a requirement above, none changes the index
  schema.
