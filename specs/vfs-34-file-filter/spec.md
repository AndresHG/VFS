# VFS-34 — File filter: excluded names and extensions, and a size limit for reading content

- **Status**: approved. The planning session relayed the owner's request and recommended the
  three choices below; the spec takes them. An independent verification pass checked it.
- **Size**: M. One subsystem: the indexer's exclusion matcher, plus one Ajustes section and
  one budget of VFS-2. There is no index schema change.
- **Links**:
  - Issue #165, Synology Drive's «Filtro de archivos».
  - [VFS-33](../vfs-33-bigroot/spec.md) R5–R8: the exclusion matcher, and the per-root
    `exclusions/<id>` record.
  - [VFS-2](../vfs-2-content-search/spec.md): the input-size budget and the `too_large`
    reason.
  - [VFS-27](../vfs-27-content-in-app/spec.md): the content run.
  - [VFS-18](../vfs-18-shortcuts/spec.md): shortcuts, which are indexed on purpose.

## Problem

The owner's archive is full of files nobody looks for: editor swap files, Office lock files
(`~$informe.docx`), half-finished downloads, `Thumbs.db`. They clutter results and cost index
space. Synology Drive, which the owner already uses, lets the user exclude files by name or
extension and skip files over a size. VFS has no such control. VFS-33 excludes folders only,
from a fixed list. With content search coming in v0.3, very large documents also cost
minutes of extraction each on the target PC for little value.

## Requirements (EARS)

- R1. THE Ajustes panel SHALL have a section «Filtro de archivos», containing:
  - the list of excluded patterns, each with a remove (×) button;
  - a field with the placeholder «Nombre o extensión (por ejemplo *.tmp o Thumbs.db)» and an
    add (+) button;
  - «Restablecer la lista»;
  - the field «Tamaño máximo para leer el contenido (MB)» (R6).
  The list is global: it applies to every folder (D2).
- R2. A pattern SHALL match a file's name only, never a folder and never the path, ignoring
  case. Its syntax: `*` stands for any run of characters and `?` for one character; any
  other character is literal. The field SHALL reject an empty pattern, one containing `/` or
  `\`, and an exact duplicate, with a short message.
- R3. WHILE a pattern matches a file, THE walker SHALL not index it, and THE watcher and
  replays (VFS-9, VFS-26) SHALL drop changes to it. The check costs one pass over the pattern
  list per listed file, with no file-system call (VFS-33 R7).
- R4. WHEN the user adds a pattern, THE indexer SHALL delete the matching files already in
  the index, in the background, in batches, with no walk. Search stops showing them as soon
  as the indexer has deleted them. Their content rows go with them (VFS-2 R3, a file gone
  from the index).
- R5. WHEN the user removes a pattern or restores the list, THE app SHALL mark every root as
  owing a full walk, through VFS-33's `exclusions/<id>` record (whose version now includes
  the user list). The walk is deferred for a root being walked (VFS-33 R8). Files no longer
  excluded come back after that walk. Ajustes says so: «Los archivos volverán a aparecer
  tras la próxima revisión completa».
- R6. THE content indexer SHALL not open a document larger than «Tamaño máximo para leer el
  contenido (MB)». Such a document stays in the name index and gets the unread reason
  `too_large` (VFS-2 R15). The default is 100 MB, VFS-2's input budget, and 0 means no
  limit. Raising the limit re-queues the `too_large` documents now within it. Lowering it
  does not remove text already read. The field applies only to content; the name index has
  no size limit (D1).
- R7. A fresh install, and «Restablecer la lista», SHALL hold the default patterns of D3.
  They never include `*.lnk`, `*.url`, or any pattern that matches a shortcut (VFS-18).

## Acceptance scenarios

Tests cite these ids (`VFS-34 AC-<m>`) in their doc comments.

- AC-1 (R2) Matcher unit tests on both platforms:
  - `*.tmp` matches `A.TMP` but not `a.tmp.txt`;
  - `~$*` matches `~$informe.docx`;
  - `Thumbs.db` matches only that name, ignoring case;
  - `?` matches one character;
  - no pattern matches a folder name or a path;
  - the add field rejects ``, `a/b`, `a\b` and a duplicate.
- AC-2 (R3) Given a tree with `nota.txt`, `nota.txt.swp`, `~$carta.docx` and `Thumbs.db`,
  and the default list, When it is scanned, Then only `nota.txt` is indexed. When a
  `borrador.tmp` is created while watching, Then it never enters the index.
- AC-3 (R4) Given an indexed `copia.bak`, When the user adds `*.bak`, Then the file is gone
  from search after the indexer's background deletion, no walk ran, and its content row is
  gone.
- AC-4 (R5) Given `*.bak` in the list and a `copia.bak` on disk, When the pattern is removed,
  Then every root owes a full walk, and after the walk `copia.bak` is found.
- AC-5 (R6) Given a 150 MB PDF and the default limit, When content indexing runs, Then the
  PDF is never opened, its name is found, and its unread reason is `too_large`. When the
  limit is raised to 200, Then the PDF is queued and read. When the limit is 0, Then no
  document is skipped for size.
- AC-6 (R1, R7) UI test: the section exists with the list, add field, remove buttons, reset
  and the size field. A fresh install shows the D3 defaults, none of which matches
  `Acceso.lnk` or `web.url`. Its copy exists in both catalogs.

## Non-goals

- Per-folder lists (D2).
- Excluding folders by pattern. Folders are VFS-33's job, and a folder can be removed from
  the roots.
- A size limit for the name index (D1).
- Regular expressions, or patterns that match a path.
- Excluding by content or by date.

## Pre-mortem

- **Slow PC / HDD.**
  - Matching adds no I/O (R3).
  - Adding a pattern deletes rows in batches and walks nothing (R4).
  - Removing one costs a full walk, deferred and in the background. Ajustes says so (R5).
  - The size limit saves extraction time on the largest files (R6).
- **Paths over 260 characters.** n/a, because patterns match the name only.
- **NFC/NFD.** Patterns and names are compared after NFC normalisation and ASCII case
  folding, as VFS-33 compares folder names.
- **Antivirus.** n/a, because no file is opened.
- **Kill / power cut.** The background deletion of R4 runs in transactions and resumes on the
  next start from the stored list. The walk owed by R5 is stored in the `exclusions/<id>`
  record, as in VFS-33 R8.
- **Huge result sets.** n/a, because this only removes results.
- **Permission-denied, locked files, cloud placeholders.** Unchanged. A cloud-only file
  that matches a pattern is simply not indexed.
- **User files.** Nothing is opened, moved or deleted on disk: "excluded" means "not in the
  index".

## Phased PRs

One PR, built after VFS-2 2a, because R6 replaces VFS-2's fixed input budget with this
setting. 2–3 days **[estimate]**. Without content search, R6's field shows but has no effect
until VFS-27 ships in the same release.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | What does the size limit apply to? | Content extraction only. Big files stay findable by name, which is the app's first promise (VFS-1). Synology's limit is about sync traffic; VFS's cost is reading. | planning session's recommendation, taken by the agent | 2026-11-22 |
| D2 | Global or per folder? | Global, for simplicity. Synology sets it per sync task, but VFS's users set it once. | planning session's recommendation, taken by the agent | 2026-11-22 |
| D3 | Default patterns | `*.tmp`, `*.temp`, `*.swp`, `~$*` (Office lock files), `.~lock.*#` (LibreOffice lock files), `*.crdownload`, `*.part`, `Thumbs.db`, `.DS_Store`, `desktop.ini`. Left out: `*.lnk` (VFS-18), and `*.pst`, `*.bak` and `*.log`, which a user may want to find by name. | agent | 2026-11-22 |
| D4 | How a change is applied | Adding a pattern deletes the matches from the index without a walk. Removing one owes a full walk, since those files were never indexed. VFS-33's per-root record carries the list's version. | agent | 2026-11-22 |

## Open questions

None.
