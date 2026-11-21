# VFS-18 — Shortcuts and linked folders

- **Status**: approved (owner, 2026-11-20); implemented: `.lnk`, `.url`, and symbolic links
  and junctions listed as link rows (AC-1 to AC-11).
- **Size**: L. It changes the index schema (a new table, a migration), opens one more kind
  of user file (the shortcut file itself, read only), behaves differently on Windows and
  macOS, adds a deferred indexer pass and a second row action.
- **Links**: [VFS-1](../vfs-1-core-search/spec.md) (search rules, contract, wording, reveal),
  [VFS-6](../vfs-6-bookmarks/spec.md) ("Abrir en Firefox"), GitHub issues 91 and 96.

## Problem

The end user keeps a large archive on a slow Windows PC and uses Windows shortcuts
("Acceso directo", `.lnk`) heavily: often a shortcut is how they remember where something
is, and some point to places VFS does not index (the NAS, a USB disk). Today a shortcut is
an ordinary file of kind "other" named `Fotos boda.lnk`; nothing reads where it points, it
cannot be found by its destination, and "Mostrar en carpeta" shows the shortcut, not the
destination. Folders that are symbolic links or junctions inside a chosen folder are
skipped entirely. Internet shortcuts (`.url`) are plain files although they hold a web
address, like the Firefox bookmarks VFS-6 made searchable.

## What we know (research, 2026-11-20)

Primary sources: [MS-SHLLINK] rev. 10.0 (learn.microsoft.com/openspecs/windows_protocols/
ms-shllink), Microsoft Learn (shell links, `IShellLink::Resolve`, reparse points, hard links
and junctions, `System.Link.TargetParsingPath`), Apple developer documentation (bookmark
data), crates.io and the crates' repositories, voidtools documentation and forum, and the
Rust standard library source. Full notes with a link per claim: `docs/research/shortcuts.md`.

- **`.lnk` layout.** A 0x4C-byte header (CLSID `00021401-0000-0000-C000-000000000046`,
  `LinkFlags` at 0x14), then optional sections in order: `LinkTargetIDList` (shell items;
  Microsoft calls their format private to their creator), `LinkInfo`, `StringData`
  (name, relative path, working directory, arguments, icon; counted, UTF-16 when
  `IsUnicode`), `ExtraData` blocks (size + signature, ended by a size below 4).
- **Where the target is.** `LinkInfo` gives `LocalBasePath` + `CommonPathSuffix` for a
  local volume (with a `VolumeID`: drive type 2 removable, 3 fixed, 4 remote, 5 CD-ROM;
  serial; label), or a `CommonNetworkRelativeLink` with `NetName` (`\\server\share`) and,
  for a mapped drive, `DeviceName` (`Z:`), + `CommonPathSuffix`. A header size of 0x24 or
  more adds Unicode copies; Windows writes them only when the path does not fit the ANSI
  code page, so Spanish names (á, ñ) are usually stored in Windows-1252. The
  `EnvironmentVariableDataBlock` (0xA0000001) holds a target with `%VARIABLES%`; the shell
  prefers it when `PreferEnvironmentPath` is set. Links with no `LinkInfo` and no
  environment block have no file-system path we can read: Control Panel and other shell
  objects, installer-advertised shortcuts (Darwin block 0xA0000006), Store apps.
- **Reading is not resolving.** A link's bytes are read with ordinary file I/O; resolution
  is a separate, explicit `IShellLink::Resolve` call that uses Distributed Link Tracking,
  searches nearby folders and volumes, can show a dialog and, with `SLR_UPDATE`, rewrites
  the `.lnk`. VFS never resolves. Opening or stat-ing a UNC path contacts the server (and
  can wait for its timeout); reading the link file that names it does not.
- **Crates.** `lnk` 0.6.4 (MIT) panics on network links (open issue 24) and pulls
  binrw/chrono/uuid; `parselnk` 0.1.1 is unmaintained and reads the whole input unbounded;
  `lnk_parser` 0.4.3 is forensics-oriented with heavy dependencies. None fuzzes beyond the
  header. **Decision D8: a hand-written bounded parser** (`crates/vfs-core/src/link.rs`,
  about 300 lines), at most 64 KiB read, every offset checked.
- **`.url` files** are INI text: `[InternetShortcut]` then `URL=…` (Microsoft documents
  the COM object, not the file layout). A few hundred bytes.
- **Symbolic links, junctions, mount points.** Junctions link folders across local
  volumes, never to a network share; symbolic links can be relative and can point to a UNC
  path; following either can loop (`C:\ProgramData\Application Data` points back to
  `C:\ProgramData`), count the same files twice, leave the chosen folder, or cross to
  another disk or the network. Rust's `FileType::is_symlink` is true for every
  name-surrogate reparse point (symlinks, junctions, mount points), so the walker never
  descends into them today; cloud placeholders are not name surrogates. `fs::read_link`
  reads the reparse data of the link itself, without touching the target.
- **macOS aliases** are bookmark data in an undocumented format, readable without
  resolution only through CoreFoundation (`CFURLCreateResourcePropertiesForKeysFromBookmarkData`).
  The end user is on Windows: out of scope (non-goals).
- **Prior art.** Everything indexes `.lnk` files by name, offers a `shortcut-target:`
  search function, and does not follow junctions (a junction can be added as its own
  folder). Windows Search exposes the target as a property column, not in its full-text
  index. Neither shows a shortcut and its target as one result.

## Design

### Components

- **`link.rs` (vfs-core, new)**: `parse_lnk`, `parse_url`, `read_target` (open, read at most
  `MAX_READ` = 64 KiB, close). Target order: environment path when the link prefers it;
  else `LinkInfo` local path; else its network path (the mapped drive letter when recorded,
  else `\\server\share`) + suffix; else the environment path; else the relative path made
  absolute against the link's folder, lexically. Windows-1252 for ANSI strings; `%VAR%`
  expanded from the app's own environment (no I/O). Targets over 32,767 characters or
  with control characters are discarded. The shell item list is skipped, never decoded.
- **Walker (`indexer/walk.rs`)**: a symbolic link or junction entry is listed as a file row
  with flag bit 1 (`LINK_ENTRY`), kind "other", size 0 and its own modification time; it is
  still never followed. Files named `*.lnk`, `*.url` and link entries get a `links` row in
  state *pending* when inserted or when their size or modification time changes.
- **Deferred pass (`indexer`)**: after the walk of every root is done and while nothing
  else is pending, the indexer worker reads pending links in id order, 200 per
  transaction, on its background-priority thread, honouring pause and shutdown, and stores
  the target (or *no target*). A `.lnk`/`.url` is read with `read_target`; a link entry with
  `fs::read_link` (no `\\?\` prefix kept; a relative target made absolute lexically). An
  unreadable file (locked, permission denied, antivirus) is marked *unreadable* and retried
  on the next full walk. Rows go away with their file row.
- **Schema (next free version at merge)**: `links(file_id INTEGER PRIMARY KEY, kind INTEGER
  NOT NULL, state INTEGER NOT NULL, target TEXT, target_key TEXT)`; `kind` 0 shortcut, 1
  internet, 2 symlink; `state` 0 pending, 1 read, 2 unreadable; `target_key` = `fold_key`
  of the target with `/` separators. No full-text index: the table holds only links (a
  few thousand rows) and is scanned. The migration creates the table and queues every
  existing `*.lnk`/`*.url` file as pending; link entries appear on the next walk.
- **Search (`search.rs`)**: a file hit that is a link carries `link: {kind, target}`.
  A link-target pass scans `links` with a target and verifies every term in the link's own
  folder path, its name or its target (same folding and one-typo rule); a hit that needed
  the target has `matched = target`. Ranking puts `target` with path matches (tier 3).
  Before the list is cut, a link hit is dropped when its target is an indexed file or
  folder that is itself among the candidates (decision D1). Type filters other than
  "Otros" exclude link hits (their target's kind is unknown without touching it); date,
  size and folder filters apply to the link file itself.
- **Commands (Tauri)**: `reveal_target(id)` reveals the target of a shortcut or link entry
  in Explorer/Finder; it runs off the UI thread (a UNC target can block for the network
  timeout) and fails with `not_found` when the target does not exist, never searching for
  it or opening another folder. `open_link_url(id)` opens a `.url` address in Firefox
  through the VFS-6 path (`http`/`https` only).
- **UI (Papel)**: a link row keeps its icon, name and "Mostrar en carpeta" (Intro). Its
  name gets a badge (`Acceso directo`, `Página web`, `Enlace`); its second line becomes
  `Lleva a: <target>` (highlighted like a path) or, for no target, stays the folder. The
  details line shows both: `Acceso directo en: <full path> · Lleva a: <target>`. A second
  button beside the first: `Mostrar el destino` for shortcuts and link entries with a
  target, `Abrir en Firefox` for an openable `.url` (keyboard: Mayús+Intro). A missing
  target shows `No se encuentra el destino «<target>». Puede que se haya movido o borrado,
  o que el disco no esté conectado.` The target is text only: never a link, image or URL in
  the page, so displaying it touches nothing.

## Requirements (EARS)

- R1. WHEN a folder is listed, THE walker SHALL record `*.lnk` and `*.url` files and
  symbolic-link or junction entries as link rows and SHALL never descend into a link entry.
- R2. WHEN the walks are done and links are pending, THE indexer SHALL read each pending
  link once in a deferred background pass, opening only the link file itself (or reading
  the link's own reparse data) and reading at most 64 KiB of it.
- R3. THE app SHALL never open, stat, resolve or contact a link's target, except when the
  user asks for "Mostrar el destino" or "Abrir en Firefox".
- R4. WHEN a search runs, THE search SHALL return a link whose every term is in its own
  folder path, its name or its target, and SHALL attach the link kind and target to every
  link hit.
- R5. IF a link's target is an indexed file or folder that the same search returns, THEN
  THE search SHALL leave the link out.
- R6. WHEN the user asks for "Mostrar el destino", THE app SHALL reveal the target in
  Explorer/Finder; IF the target does not exist, THEN it SHALL show the missing-target
  message and do nothing else.
- R7. WHEN the user asks for "Abrir en Firefox" on an Internet Shortcut, THE app SHALL open
  its address in Firefox, only for `http`/`https` addresses as for bookmarks.
- R8. THE app SHALL never write to, repair, move or delete a shortcut or link.
- R9. IF a link file is malformed, truncated, hostile, absurdly large or unreadable, THEN
  THE indexer SHALL store no target for it, keep indexing, and never panic or allocate
  more than the bounded read.
- R10. THE index SHALL migrate from the previous schema version without a full rescan.

## Acceptance scenarios

Tests cite these ids (`VFS-18 AC-<m>`).

- AC-1 (R1, R2) Given a root with `a.lnk`, `b.URL`, a symbolic link to a folder and an
  ordinary file, When it is indexed, Then the three links have pending link rows, are read
  by the deferred pass after the walk, and the folder behind the symbolic link is not
  listed.
- AC-2 (R2) Given real shortcuts written by the Windows shell to a local file, a local
  folder, a network path, a drive letter that is not present, a target deleted after the
  link was made, a path over 240 characters, targets in and outside the ANSI code page,
  and another shortcut, When they are read, Then each target is the path the shell was
  given (for a link made to a shortcut the shell itself records the final target; VFS reads
  only what is recorded and never opens the first shortcut).
- AC-3 (R3) Given a shortcut whose target is a UNC path on an unreachable server, When the
  root is indexed and searched and the hit is displayed, Then no file or network access is
  made to the target (the target is never passed to a file-system call).
- AC-4 (R4) Given `Familia\Fotos.lnk` → `D:\Boda 2003`, When searching `boda`, `familia
  boda` and `fotos`, Then the shortcut is a hit each time, with `matched` = `target` for the
  first two and `name` for the third, and carries kind `shortcut` and its target.
- AC-5 (R5) Given a shortcut whose target is an indexed file, When a search finds both,
  Then only the file is returned; When a search finds only the shortcut (by its own folder
  or name), Then the shortcut is returned.
- AC-6 (R6) Given a hit whose target exists, When "Mostrar el destino" is asked, Then the
  target is revealed; Given a missing target or an absent drive, Then the command fails
  with `not_found` and the UI shows the missing-target message.
- AC-7 (R7) Given `Banco.url` with an `https` address and `x.url` with a `file:` address,
  When searched by a word of the address, Then both are hits with kind `internet`; only the
  first offers "Abrir en Firefox".
- AC-8 (R1) Given symbolic links forming a loop (`a/b` → `a`) and a link to outside the
  root, When indexed, Then every real file is listed exactly once, the walk ends, and the
  links are rows with their targets.
- AC-9 (R9) Given empty, truncated, randomly corrupted, 10 MB and absurd-offset link files,
  When read, Then each yields a target or none, never a panic, reading at most 64 KiB.
- AC-10 (R10) Given an index at the previous schema version holding `x.lnk`, When opened,
  Then the `links` table exists with `x.lnk` pending and no rescan is queued.
- AC-11 (R8) The link code paths open link files read-only; a test asserts the link file's
  bytes and modification time are unchanged after indexing and both actions.

## Non-goals

- Resolving moved targets (Distributed Link Tracking, search heuristics) or repairing links.
- Following links into their targets (decision D5): a linked folder's contents are not
  searchable unless its target is added as a folder of its own.
- macOS Finder aliases (CoreFoundation-only format; the end user is on Windows): they stay
  ordinary files. macOS symbolic links are covered.
- Decoding shell item lists (Control Panel items, Store apps, advertised shortcuts): such a
  link is a link row with no target.
- Classifying a link by its target's kind for the type filter.
- Checking whether a target exists while showing results.

## Pre-mortem

- Slow PC, busy HDD: one extra open + read of 0.5–4 KB per link, deferred until the walks
  are done, on the background-priority thread, resumable (pending rows survive a restart);
  measured cost per shortcut on the Windows runner (Defender on) is in the PR.
- Long paths: targets up to 32,767 characters are kept; link files are opened through the
  same long-path handling as the walk; "Mostrar el destino" uses the VFS-1 reveal planning
  (`\\?\` and the Explorer limit).
- NFC/NFD: targets go through `fold_key` like names.
- Antivirus: opening a `.lnk` may be scanned and slowed or denied; the link is marked
  unreadable and retried on the next full walk; reading never holds the file open.
- Killed mid-write: the pass writes in transactions of 200; pending rows remain pending.
- A query matching a million files: the target pass scans only `links` and adds at most its
  matches to the same capped, ranked list.
- Permission-denied, locked, symlinks, junctions, cloud placeholders: links are listed,
  never followed; placeholders unchanged (skipped by VFS-1 rules); unreadable links have no
  target.
- Opening or writing user files: only link files are opened, read-only, bounded (R2, R8).

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Show shortcuts in results? | Only when they add something: hidden when the same search already returns the target; shown when the target is outside the chosen folders, missing, or not found by the search itself ("if the real file is there, I don't know what's the benefit"; accepted the proposal) | owner | 2026-11-20 |
| D2 | How a row shows it is a shortcut | Badge + `Lleva a: <target>` line; nothing checked on the target disk | owner | 2026-11-20 |
| D3 | Second action, missing target | `Mostrar el destino`; if missing, explain (moved, deleted or disk not connected); never search or repair | owner | 2026-11-20 |
| D4 | Findable by target name and path | Yes, by own folder, name and target together ("one folder may be in the search criteria, and the other may not") | owner | 2026-11-20 |
| D5 | Symbolic links and junctions inside a root | Shown like shortcuts, never entered | owner | 2026-11-20 |
| D6 | `.url` files | Like bookmarks: file results showing the address, `Abrir en Firefox`, found by name and address | owner | 2026-11-20 |
| D7 | When targets are read | Deferred pass after the walks, not during them | session | 2026-11-20 |
| D8 | Parser | Hand-written, bounded; no crate (research above) | session | 2026-11-20 |
| D9 | Row layout | Target replaces the folder on the second line (fixed row height); both in the details line; second button beside the first; Mayús+Intro | session | 2026-11-20 |
| D10 | Type filter on links | Only "Otros" keeps them | session | 2026-11-20 |

## Open questions

None.
