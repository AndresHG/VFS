# VFS-34 — File filter: excluded names and extensions, and a size limit for reading content

- **Status**: approved. The planning session relayed the owner's request and recommended D1
  and D2, which the spec takes. An independent critic review asked for revisions; all of
  them are made here.
- **Size**: M. One subsystem: the indexer's exclusion matcher, plus one Ajustes section and
  one setting read by VFS-2. There is no index schema change: the list and the setting are
  `meta` keys.
- **Links**:
  - Issue #165, Synology Drive's «Filtro de archivos».
  - [VFS-33](../vfs-33-bigroot/spec.md) R5–R8: the exclusion matcher, and the per-root
    `exclusions/<id>` stamp.
  - VFS-1 I8: the built-in list `walk::is_excluded`, used by the walker and the watcher.
  - [VFS-2](../vfs-2-content-search/spec.md) R3, R15 and R21: the size setting, `too_large`,
    and the sweep of removed files.
  - [VFS-27](../vfs-27-content-in-app/spec.md): the content run.
  - [VFS-18](../vfs-18-shortcuts/spec.md): shortcuts, which are indexed on purpose.

## Problem

The owner's archive is full of files nobody looks for: editor swap files (`*.swp`),
half-finished downloads (`*.crdownload`, `*.part`), application temporary files (`*.tmp`).
VFS already ignores a fixed set of system and lock files (`Thumbs.db`, `.DS_Store`,
`desktop.ini`, `~$*`, `.~lock.*#`, `~*.tmp`, `._*`; VFS-1 I8). The user cannot add to that
set. Synology Drive, which the owner already uses, lets the user exclude files by name or
extension and skip files over a size. With content search coming in v0.3, very large
documents also cost minutes of extraction each on the target PC for little value.

## Requirements (EARS)

- R1. THE Ajustes panel SHALL have a section «Filtro de archivos», global for every folder
  (D2), with:
  - the user's list of excluded patterns, each with a remove (×) button;
  - a field with the placeholder «Nombre o extensión (por ejemplo *.tmp)» and an add (+)
    button;
  - «Restablecer la lista»;
  - a line naming what VFS always ignores, which the user cannot remove: «VFS siempre ignora
    los archivos de sistema y de bloqueo: Thumbs.db, .DS_Store, desktop.ini, ~$…» (D3).
  WHILE content search is on (VFS-27), the section SHALL also show the field «Tamaño máximo
  para leer el contenido (MB)» (R6).
  The list is stored in `index.db` `meta` key `user_exclusions`, as one JSON array.
- R2. A pattern SHALL match a file's name only, never a folder and never a path. Matching
  compares both sides after NFC normalisation and Unicode case folding. `*` stands for any
  run of characters and `?` for one character; any other character is literal. The field
  SHALL reject, with a short message:
  - an empty pattern;
  - one containing `/` or `\`;
  - one with no literal character (`*`, `*.*`, `?*`);
  - an exact duplicate.
  Before adding, it SHALL show how many indexed files the pattern matches, and ask for
  confirmation when that number is above 1,000.
- R3. WHILE a pattern matches a file, THE walker and the folder lister SHALL not index it. The
  watcher is not changed: a change event triggers the re-listing of its folder, and the
  listing applies the patterns. The check costs one pass over the pattern list per listed
  file, with no file-system call (VFS-33 R7). The list reloads into the indexer's filter
  when it changes, as VFS-33's `include_all` does.
- R4. WHEN the user adds a pattern, THE indexer SHALL delete the matching files already in
  the index in the background, with no walk:
  - It scans `files` in `id` order in batches of 10,000, matching in Rust.
  - It keeps the `meta` keys `user_exclusions_purge` (the "purge owed" flag) and
    `user_exclusions_cursor` (the last `id` done), so a restart resumes where it stopped.
    Both are removed when the scan reaches the end.
  - Search stops showing a file as soon as its batch is deleted.
  - Their content rows are removed by the next content run (VFS-2 R21).
- R5. VFS-33's per-root stamp `exclusions/<id>` SHALL gain a generation number for the user
  list, giving `VERSION:include:gen`. Removing a pattern or restoring the list bumps the
  generation, and adding a pattern does not. A stamp in the old `VERSION:include` form reads
  as generation 0, the generation of a fresh list. Upgrading therefore makes no root stale. A bumped generation makes every root owe a
  full walk (VFS-33 R8, deferred for a root being walked), and files no longer excluded come
  back after that walk. Ajustes says so: «Los archivos volverán a aparecer tras la próxima
  revisión completa».
- R6. THE content indexer SHALL not open a document larger than «Tamaño máximo para leer el
  contenido (MB)», stored in `index.db` `meta` key `content_max_mb` and read by
  `vfs-content` (VFS-2 R21). Such a document stays in the name index, and its unread reason
  is `too_large` (VFS-2 R15).
  - The default is 100 MB, VFS-2's input budget, and 0 means no limit.
  - Raising the limit re-queues the `too_large` documents now within it (VFS-2 R3's
    exception).
  - Lowering it does not remove text already read.
  The name index has no size limit (D1).
- R7. A fresh install, and «Restablecer la lista», SHALL hold the default patterns of D3.
  The user list never ships `*.lnk`, `*.url` or any pattern that matches a shortcut
  (VFS-18).

## Acceptance scenarios

Tests cite these ids (`VFS-34 AC-<m>`) in their doc comments.

- AC-1 (R2) Matcher unit tests on both platforms:
  - `*.swp` matches `A.SWP` but not `a.swp.txt`;
  - `?` matches exactly one character;
  - a pattern with `É` matches a name stored in NFD;
  - no pattern matches a folder name or a path;
  - the add field rejects ``, `a/b`, `a\b`, `*`, `*.*`, `?*` and a duplicate;
  - adding a pattern that matches 1,500 indexed files asks for confirmation first.
- AC-2 (R3, R7) Given a tree with `nota.txt`, `nota.txt.swp`, `video.mp4.part` and
  `informe.pdf.crdownload`, and the default list, When it is scanned, Then only `nota.txt`
  is indexed. When a `borrador.swp` is created while watching, Then the re-listing does not
  index it. A folder named `fotos.part` and its files are indexed.
- AC-3 (R4) Given 25,000 indexed files of which 12,000 are `*.bak`, When the user adds
  `*.bak`, Then they are gone from search after the background purge, no walk ran, and no
  `exclusions/<id>` stamp went stale. When the app is killed after the first batch and
  restarted, Then the purge resumes from its cursor and finishes.
- AC-4 (R5) Given `*.bak` in the list and a `copia.bak` on disk, When the pattern is removed,
  Then the generation is bumped and every root owes a full walk. After the walk,
  `copia.bak` is found.
- AC-5 (R6) Given a 150 MB PDF and the default limit, When content indexing runs, Then the
  PDF is never opened, its name is found, and its unread reason is `too_large`. When the
  limit is raised to 200, Then the PDF is queued and read. When the limit is 0, Then no
  document is skipped for size.
- AC-6 (R1, R7) UI test:
  - the section exists with the list, the add field, the remove buttons, reset and the line
    about built-ins;
  - the size field is hidden while content search is off and shown when it is on;
  - a fresh install shows the D3 defaults, none of which matches `Acceso.lnk` or `web.url`;
  - the copy exists in both catalogs.

## Non-goals

- Per-folder lists (D2).
- Making the built-in system and lock-file list editable (D3).
- Excluding folders by pattern. Folders are VFS-33's job, and a folder can be removed from
  the roots.
- A size limit for the name index (D1).
- Regular expressions, or patterns that match a path.

## Pre-mortem

- **Slow PC / HDD.**
  - Matching adds no I/O (R3).
  - Adding a pattern purges in batches with a resumable cursor, and walks nothing (R4).
  - Removing one costs a full walk, deferred and in the background, and Ajustes says so
    (R5).
  - A pattern that matches everything cannot be added (R2).
  - The size limit saves extraction time on the largest files (R6).
- **Paths over 260 characters.** n/a, because patterns match the name only.
- **NFC/NFD.** Both sides are NFC-normalised and case-folded before matching (R2).
- **Antivirus.** n/a, because no file is opened.
- **Kill / power cut.**
  - The purge's cursor and flag resume it (R4).
  - The walk owed by R5 lives in the stamp, as in VFS-33 R8.
- **Huge result sets.** n/a, because this only removes results.
- **Permission-denied, locked files, cloud placeholders.** Unchanged. A cloud-only file
  that matches a pattern is simply not indexed.
- **User files.** Nothing is opened, moved or deleted on disk: "excluded" means "not in the
  index".

## Phased PRs

One PR, built after VFS-2 2a: R6 needs VFS-2 R21's `content_max_mb`. 2–3 days
**[estimate]**.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | What does the size limit apply to? | Content extraction only. Big files stay findable by name, which is the app's first promise (VFS-1). Synology's limit is about sync traffic; VFS's cost is reading. | planning session's recommendation, taken by the agent | 2026-11-22 |
| D2 | Global or per folder? | Global, for simplicity. Synology sets it per sync task, but VFS's users set it once. | planning session's recommendation, taken by the agent | 2026-11-22 |
| D3 | Built-ins and defaults | `walk::is_excluded` stays fixed and is not editable: removing a system or lock-file pattern would only add noise. Ajustes names it (R1). The user list's defaults are the temporary and partial-download kinds it does not cover: `*.tmp`, `*.temp`, `*.swp`, `*.crdownload`, `*.part`. Left out: `*.lnk` (VFS-18), and `*.pst`, `*.bak` and `*.log`, which a user may want to find by name. | agent | 2026-11-22 |
| D4 | How a change is applied | Adding a pattern purges the matches from the index without a walk. Removing one bumps the user-list generation in VFS-33's stamp and costs a full walk, because those files were never indexed. | agent | 2026-11-22 |

| D5 | Where the section sits | Inside Ajustes › «Avanzado», after VFS-33's switch: almost nobody needs it, and it keeps the panel short for non-technical users. | agent | 2026-11-22 |
| D6 | Reading R2's edge cases | `.` is not a literal character, so `*.*` is refused as the spec lists. A duplicate is equal after NFC and case folding (`*.TMP` repeats `*.tmp`). The pattern is trimmed. The count is shown when the pattern is added; above 1,000 a dialog asks first. | agent | 2026-11-22 |
| D7 | Indexes from before VFS-34 | When the indexer first starts, it stores the defaults and owes their purge (R4), so old `*.tmp`-style rows leave with no walk. The generation stays 0, so no root goes stale (R5). | agent | 2026-11-22 |
| D8 | The size field before VFS-27's switch exists | The field ships hidden; the UI exposes `showContentLimit(row, on)` for 27c's switch to call. The setting itself already works (`content_max_mb`), and 0 means no limit in `vfs-content`. | agent | 2026-11-22 |

## Open questions

None.
