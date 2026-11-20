# Prior art: how existing file-search tools work, and where VFS stands

Study date: 2026-11-20. Scope: tools that find files by name and path on a desktop, plus the
document-content tools whose ideas matter for Fase 3. VFS is compared as designed on the phase-1
branch (`feat/vfs-1-release`, PR 61: `docs/architecture.md`, `docs/decisions.md`,
`specs/vfs-1-core-search/spec.md`). Windows internals (USN, MFT, priorities, Defender) are in
[`windows-io.md`](windows-io.md) and [`windows-recommendations.md`](windows-recommendations.md) and
are not repeated here. The ranked proposals that follow from this study are in
[`prior-art-proposals.md`](prior-art-proposals.md).

Evidence tags, as in the rest of `docs/research/`:

- **[doc]** official documentation, changelog, licence file or source code read at a stated commit;
- **[author]** the tool's author or vendor speaking (forum post, README claim, marketing);
- **[3p]** third-party report or measurement (forum user, review, issue tracker);
- **[est]** our own inference or arithmetic, with the reasoning given;
- **[measured]** a VFS figure measured on the phase-1 branch (development Mac, synthetic tree), as
  recorded in its `docs/architecture.md` and `docs/decisions.md`.

No other tool was installed or run for this study. Source code was read
(Kensaku, FSearch, Cardinal, ripgrep-all, fd); Reddit could not be fetched, so user complaints come
from the tools' own forums and issue trackers, review sites and Apple/Microsoft community pages.

## 1. Summary

1. **The fast name-search tools are RAM-resident scanners, not inverted indexes.** Everything,
   FSearch and Cardinal keep a compact copy of every name in memory and scan it in parallel on
   every keystroke, cancelling on the next one. Their published or estimated cost is 60–100 MB of
   RAM per million files ([§2.1](#21-voidtools-everything), [§2.7](#27-fsearch),
   [§2.8](#28-cardinal)). VFS instead keeps an on-disk SQLite FTS5 trigram index, which costs more
   disk (889 MB at 2 M files, measured on the phase-1 branch) but little RAM and survives a crash.
2. **Their enumeration speed comes from privileges, not from the scan.** Everything reads the NTFS
   master file table through a SYSTEM service; WizFile and WizTree read it as administrator. Every
   one of them falls back to an ordinary directory walk without admin, and their vendors call that
   path "much slower". VFS is permanently in that fallback regime ([§4](#4-everything-against-vfs)).
3. **What makes them feel instant after the first load is a saved change cursor.** Everything
   replays the USN journal from where it stopped; Cardinal resumes FSEvents from a stored event id.
   Neither re-walks the tree at start. VFS re-walks every root at every start (spec I4), which this
   study identifies as the main design risk ([§5](#5-what-the-study-says-about-the-vfs-design)).
4. **Nobody else does typo tolerance on file names, and accent folding is mostly wrong.**
   Everything ignores diacritics by default but its built-in Audio/Video filters do not; FSearch
   folds only one way (`cafe` finds "café", `café` does not find "cafe"); Cardinal only unifies
   NFC and NFD; Kensaku has no accent folding for content and probably drops accented names in its
   name prefilter. Windows Search defaults to word-prefix, not substring. VFS's two-way fold plus
   one-typo tolerance is genuinely ahead ([§3](#3-comparison-table)).
5. **The OS indexers cannot carry this archive.** Windows Search documents problems above
   400,000 items and a practical cap of 1 million; Spotlight re-indexes take hours to days and do
   not reach network shares ([§2.9](#29-windows-search), [§2.10](#210-macos-spotlight-and-mdfind)).
6. **For content (Fase 3) the transferable ideas are about failure handling**: per-file timeouts in
   an isolated process, a visible list of skipped files (Recoll users cannot find theirs), an
   extraction cache invalidated by extractor version (ripgrep-all), and indexing the name even
   when the content fails ([§2.4](#24-recoll)–[§2.6](#26-ripgrep-all-and-fd)).
7. **Reusable code is scarce.** Kensaku (MIT) and Cardinal (MIT) can be copied with attribution;
   Everything's SDK and `es` are MIT but useless without Everything installed; Recoll, Xapian and
   FSearch are GPL and ripgrep-all is AGPL, so only their ideas can be used.

## 2. The tools

### 2.1 voidtools Everything

**Versions.** Stable 1.4.1.1032 (23 Jan 2026). 1.5 left alpha with 1.5.0.1410b (14 May 2026); the
newest build is 1.5.0.1424b (15 Oct 2026), which made the service check that it is trusted and
limited IPC to clients in the same Windows session **[doc]** [E1][E2][E3].

**Enumeration and freshness.**
- On NTFS (and ReFS in current builds) Everything reads the MFT, keeps "a very light copy" in
  memory and follows the USN change journal; changes made while it was closed are replayed at the
  next start because Windows keeps the journal **[author]** [E4], **[doc]** [E5].
- If the journal wrapped while it was closed, it rebuilds the volume ("Out of date NTFS
  volume…"); void advises raising the journal maximum from 32 MB to 256 MB **[author]** [E6].
- The **Everything Service** is installed by default, starts with Windows, uses about 1 MB of RAM,
  and lets a standard user index NTFS. Only a voidtools-signed `Everything.exe` may connect to it,
  and it gives "file names only" — to any local user, which is a privacy point **[doc]** [E7]. The
  service account is not documented (LocalSystem is assumed **[est]**).
- Without the service the choices are running elevated (a UAC prompt every start) or **folder
  indexing**, which the docs call "much slower" **[doc]** [E5][E7].
- **Folder indexing** is used for FAT, exFAT, mapped drives and NAS shares. It walks with
  `FindFirstFile` (1.5 adds a large-fetch option and a `folder_scan_sleep` throttle), monitors with
  `ReadDirectoryChanges` (64 KB buffer, capped at 64 KB on shares, "rescan on full buffer"), and
  by default **rescans daily**; a missed rescan runs at the next start **[doc]** [E8], **[author]**
  [E9]. A 2026 user report: a folder-index rescan of a QNAP share made local Excel saves take
  minutes until Everything was closed; void's background-I/O option "can prevent Everything from
  ever completing a rescan" **[3p][author]** [E10].
- Offline volumes stay searchable if "automatically remove offline volumes" is unticked **[doc]**
  [E11].

**Index structure and size.**
- Undocumented. void describes 1.5's gains as "tightened loops" that keep hot data in CPU cache,
  and compressing the parent pointer cut RAM little and cost about 16× CPU **[author]** [E12][E13]:
  consistent with a parent-linked array of names scanned linearly **[est]**.
- Precomputed **fast sort** orders for name, run count, recently changed (always) and optionally
  size, dates, path, extension; size costs 8 bytes per entry and its fast sort 16 more
  **[doc]** [E11], **[author]** [E14]. Without a size index a size sort "will take minutes" **[3p]** (voidtools forum, <https://mail.voidtools.com/forum/viewtopic.php?p=13677>).
- **Memory:** 1 M files ≈ 100 MB RAM and 45 MB on disk **[doc]** [E5]; "around 100MB per 1
  million files" **[author]** [E15]; "700MB is expected for 8 million files" **[author]** [E14];
  a 30 M-item index used 6–10 GB **[3p]** [E15]. At 5 M files: about 0.5 GB RAM and 225 MB on disk
  **[est]**.
- **Persistence:** 1.4 writes `Everything.db` only on exit, so an unclean shutdown loses the
  session and often triggers a rebuild; 1.5 adds a daily auto-save **[author]** [E16][E17].
- Build speed: about 1 minute per million files on the MFT path **[doc]** [E5] (presumably SSD).

**Query model.**
- Space = AND, `|` = OR, `!` = NOT, quotes, `< >` grouping, `*`/`?` wildcards, `regex:`
  **[doc]** [E18].
- Modifiers: `case:`, `diacritics:`, `path:`, `ww:`, `prefix:`, `suffix:`, `ignore-punctuation:`,
  `file:`/`folder:`, `soundex:`, `pinyin:`, each negatable with `no-` **[doc]** [E19].
- Functions: `ext:`, `size:` (`>1gb`, `10mb..50mb`), `dm:` (`today`, `yesterday`, `7days`,
  `2025-07-01..2026-06-30`, `>2022`), `parent:`, `child:`, `empty:`, `dupe:`, `sizedupe:`,
  `content:` and, in 1.5, hundreds of properties **[doc]** [E18][E20].
- Filter macros (`audio:`, `doc:`, `image:`, `video:`), bookmarks (search + filter + sort + view),
  search history, a configurable home search **[doc]** [E18].
- **Ranking:** none by default — results are sorted by a column (name by default); 1.5 adds
  "weighted searches" **[doc]** [E21].
- **Names only by default:** a word matches the path only with `path:` or when the query holds a
  `\` **[doc]** [E18].
- **Accents:** ignored by default (toggle with Ctrl+M) **[author]** [E22], but the built-in
  Audio/Video filters ship with diacritics on, which confused a user **[3p]** [E23]. **Typos:** no
  edit-distance tolerance; only opt-in `soundex:` and pinyin **[doc]** [E19].

**1.5 additions.** Property indexing, folder sizes (on by default), duplicates, dark mode, preview
pane, sidebars, an index journal, Everything Server (index near a NAS, query remotely), plugins,
and Spanish (Latin American in 1408a, Spain in 1409a, volunteer-translated) **[doc]** [E21],
**[author]** [E24]. Content indexing is off by default, kept in RAM, and "indexing over 1GB of text
is not recommended" **[author]** [E25].

**SDK and licence.** SDK 2 (1.4) is a `WM_COPYDATA` wrapper that "requires Everything client to be
running"; SDK 3 (1.5) uses named pipes; both MIT, as is the `es` command line **[doc]** [E26][E27].
Everything itself carries an MIT-style notice plus PCRE's BSD notice, and void confirmed
redistribution is allowed **[doc]** [E28], **[author]** [E29]. The binaries are closed source.
Bundling is legal but the service still needs an administrator to install. Windows only.

**Recurring complaints.** Unexpected rebuilds (journal wrap, unclean exit, drive changes) [E6][E16];
RAM at tens of millions of files [E15]; slow, disruptive NAS folder indexing [E10]; service and
privilege friction, incompatible 1.4/1.5 services [E2]; a dense, dated UI that overwhelms novices
[E30][E31]; no typo tolerance; content indexing crashes from third-party iFilters [E25]. **[3p]**

### 2.2 Kensaku

**Identity.** <https://github.com/yjid/kensaku>, read at commit `b8b009f` (v1.0.18,
2026-11-17). MIT. Created 2026-10-24; 622 commits, 482 by one author, 19 releases, 1.9 k stars
**[doc]** [K1].

**Architecture** (paths relative to the repo at that commit) **[doc]**:
- SQLite in WAL mode for metadata (`nodes`, `usage`, `units`, `meta`; no FTS5),
  `src-tauri/src/indexer/metadata_db.rs:71-128`.
- An in-memory arena of 64-byte nodes rebuilt from SQLite at start (`indexer/arena.rs:6-19`).
- Tantivy for document content (BM25 plus a trigram field), `indexer/content.rs`.
- Embedding vectors as i8 in SQLite, searched by an exact parallel scan, no ANN index
  (`embed/units.rs:104`, `embed/scanner.rs:57`).
- On Windows the indexer runs inside the GUI process; on Unix in a daemon.

**Indexing.** `jwalk` with one extra `metadata()` per file (`walker.rs:361`); the whole walk is
collected into one `Vec` before processing (`indexer/mod.rs:803`); a cancelled walk returns
nothing, so a partial list never reads as deletions (`walker.rs:276-281`). Change detection by
(mtime, size); restarts are full walks. `notify` watcher with a 400 ms settle window and a
rescan-with-cooldown on overflow (`watcher.rs:49,231,402-449`). Cloud placeholders
(`FILE_ATTRIBUTE_OFFLINE`, `RECALL_ON_DATA_ACCESS`) are indexed by name only and never read
(`walker.rs:85-110,188-210`). Extraction: up to 8 workers, per-file timeout 15 s (45 s OCR, +2 s
per 5 MB, cap 120 s), `catch_unwind` per extractor, bounded channel, commits every 512 files with
Tantivy before SQLite so a crash leaves files pending, never forgotten (`extract/mod.rs:428-433`,
`indexer/mod.rs:1243-1252`) **[doc]**.

**Query.** A small grammar (`type: ext: in: path: modified: size: name: content:`, quotes, `-`);
unknown `key:` is literal text and parse errors come back as hints. Name tier: `nucleo-matcher`
fuzzy subsequence scan over the arena with a 64-bit character bloom prefilter. Results from name,
BM25, semantic and look-alike tiers are fused with **weighted reciprocal rank fusion (k = 60)**,
weights chosen by query type, then a bounded additive score: +0.20 name contains the query, +0.15
name starts with it, recency and "frecency" (opens) bonuses, a depth penalty, ties broken by path
(`search/mod.rs:1265-1490,1682-1725`) **[doc]**. Inferred filters appear as removable chips.

**Accents and scale.** No ASCII folding on the BM25 analyzer (`tokenizer.rs:161-166`); the name
bloom maps non-ASCII to hashed bits, so `cafe` probably never reaches "café" **[est from code]**.
Its own benchmark gives p50 207 ms for name search at 1 M entries and says an index is needed past
1 M **[doc]**; at 5 M files the arena and walk vector would each need about 1 GB **[est]**.

**Quality.** 615 tests and a golden relevance set (43 queries, Recall@5 = 1.0,
`tests/golden_set.rs`); CI on Ubuntu only; a run of crawl-correctness bugs fixed in the last
weeks (re-extracting the whole corpus on restart, rollback on interruption, Windows folder renames
losing every file inside: issues 152, 158, 165, 187) **[doc]**. File identity is `(0,0)` on
Windows although its architecture doc says otherwise (`walker.rs:64-75`).

**Licences.** MIT throughout; `nucleo-matcher` is MPL-2.0 (file-level). Models are downloaded,
SHA-pinned: potion-base-8M (MIT), PP-OCRv6 tiny (Apache-2.0), bge-small (MIT); Indra OCR weights
are under a modified OpenRAIL-M licence and should not be shipped **[doc]**.

**Verdict for VFS.** Not a base to build on at this scale, but a good quarry: cloud-placeholder
detection, the walk-cancel rule, the watcher policy, the final-score shape, the golden-set test
harness, and (Fase 3/5) its extractors and static i8 embeddings.

### 2.3 Agent Ransack / FileLocator Pro (Mythicsoft)

- **Model:** searches the live file system on demand, no index by default; FileLocator 2022 added
  optional indexes, which cannot do regex because they are sorted term lists **[author]** [C1].
- **Syntax:** AND, OR, NOT, NEAR:n (default 100 characters), LIKE (three fuzziness levels), REGEX,
  LINES, FILELIST; operators must be uppercase or they are search words; `< >` word boundaries
  **[doc]** [C2].
- **Dates:** an anchor (Today, start/end of week, month, year; Now) plus or minus N units — "start
  of week −1 week" is "since the start of last week" **[doc]** [C3].
- **Saved searches:** `.srf` files that reopen a search, Favorites with descriptions and tags
  **[doc]** [C4].
- **Privileges:** a portable zip "for installing without Admin privileges" **[doc]** [C5].
- **Users:** praised on folders; a 50 TB user says it "struggles" on the whole archive and pairs it
  with Everything for names **[3p]** [C6]. Proprietary; nothing reusable.

### 2.4 Recoll

- **Version** 1.46.0 on Unix and Windows (GPL; Xapian GPL v2+) **[doc]** [C7].
- **Freshness:** incremental batch runs (cron / Task Scheduler) or a real-time monitor with a
  30 s minimum flush gap and per-pattern extra delays for files that change often **[doc]** [C8].
- **Index:** Xapian, unstemmed terms with stemming databases on the side; for mixed data "close to
  the size of the source data" **[doc]** [C8] — unaffordable for 4–5 TB.
- **Query:** implicit AND, uppercase OR, `-term`, phrases with slack and modifiers, `dir:`, `ext:`,
  `size:`, `date:` (ISO intervals), `mime:`; BM25-family ranking **[doc]** [C8].
- **Accents:** the default index is stripped (unaccented, lowercased) — "camion" finds "camión";
  an optional raw index switches to accent-strict when the query carries an accent **[doc]** [C8].
- **Failure handling:** `filtermaxseconds` (helper timeout, default 1200 s), `filtermaxmbytes`,
  `textfilemaxmbs` (20 MB), `indexallfilenames` (names indexed even when content is skipped, on by
  default), and `recollindex --diagsfile` with seven fixed categories per file (Skipped,
  NoContentSuffix, MissingHelper, Error, NoHandler, ExcludedMime, NotIncludedMime). Failed files
  are retried only on request or when helpers change **[doc]** [C8][C9]. Users still report not
  finding where skipped files are logged **[3p]**.
- **Complaints:** Windows crashes in the indexer and config dialog, an index unreadable after an
  upgrade, I/O load that "slows system to a crawl" **[3p]** [C10].

### 2.5 DocFetcher and AnyTXT Searcher

- **DocFetcher** 1.1.27 (EPL, Java/Lucene; development moved to the paid Pro 1.23). Per-folder
  indexes, live watching while open and a small daemon that notes which indexes need updating
  while closed; a portable index can live next to the documents **[doc]** [C11]. "A good rule of
  thumb is 200 files per minute" **[doc]** — 17 days for 5 M files **[est]**. Pro 1.12 cut the open
  time of a 2.8 M-file index from 63 s to 8.7 s **[author]** [C12]: index open time matters at
  this scale. Complaints: out-of-memory on big PDFs and Excel files (in-process extraction, no
  timeout), high CPU when watching busy folders **[doc][3p]** [C11][C13].
- **AnyTXT** 1.3.3785, closed freeware. "Almost in 0.1 second" is an unbenchmarked claim; the
  speed is a precomputed index. A LocalSystem service (`ATService`) does the indexing; one user
  saw 50 % CPU and 0.6 GB RAM for 31 minutes on the first run **[author][3p]** [C14]. Installing a
  service needs admin **[est]**.

### 2.6 ripgrep-all and fd

- **ripgrep-all** v0.10.10 (AGPL-3.0). No index: adapters (zip, tar, pandoc, `pdftotext`, ffmpeg)
  turn files into text, which ripgrep searches. The cache (`src/preproc_cache.rs`) is SQLite keyed
  by (config hash, adapter, adapter version, active adapters, path) and valid only if the stored
  mtime matches — size is not checked; values are zstd text, outputs over 2 MB are not cached, and
  a schema mismatch wipes the cache **[doc]** [C15]. Top complaint: external tools missing or
  failing (poppler broken pipe) **[3p]**.
- **fd** v10.5.2 (MIT/Apache-2.0). Regex by default, smart case, hidden and gitignored files
  skipped. Speed: the `ignore` crate's parallel walk, batched results on a bounded channel, 100 ms /
  1,000-entry buffer before streaming unsorted **[doc]** [C16]. Its 4 M-file benchmark (0.85 s) is
  warm-cache **[author]**; a maintainer notes "likely not much benefit past something like 8
  threads" and a WSL user found the default thread count far slower than one thread **[3p]**
  [C17] — on a busy HDD, more walker threads mostly add seeks **[est]**.

### 2.7 FSearch

Linux, C/GTK, GPL-2.0, version 0.3 (database rewrite), master read at 2026-11-16 **[doc]** [O16].
- Entries are variable-length records: parent pointer, flags, name inline; paths are rebuilt by
  walking parents.
- One chunked, pre-sorted array per sort order, so changing the sort is free.
- Search is a parallel linear scan: worker threads claim chunks through an atomic counter, check
  cancellation per entry, and concatenate per-chunk results in order. No inverted index.
- About 90 MB per million files with four sort orders **[est from struct sizes]**.
- Accents: ICU case folding plus NFD on both sides, combining marks kept — half folding. No typos.
- inotify/fanotify monitoring and per-folder rescan schedules since 0.3.

### 2.8 Cardinal

macOS, **Rust + Tauri 2, MIT**, version 0.1.23, 1.8 k stars, master read at 2026-06-26 **[doc]**
[O17]. The codebase closest to VFS's stack.
- First run walks the root after Full Disk Access is granted (no admin). The walk stores the
  FSEvents id taken **before** scanning; later launches resume the watcher from the persisted
  `last_event_id`, so changes made while closed are replayed without a rescan; only
  `MustScanSubDirs` or dropped events rescan a subtree.
- Index: a slab of compact nodes (name pointer, parent, children, size and times packed) plus a
  `NameIndex` from each **unique basename** to the nodes carrying it. Persisted with postcard +
  zstd, written atomically, schema-versioned.
- Search scans the set of unique names (not every file), then expands matches to nodes — a cheap
  win in archives full of `IMG_0001.JPG` and `Thumbs.db`. About 60–80 MB per million files
  **[est]**.
- Everything-like syntax (`;` OR, `!` NOT, `in:`, `size:`, `/` path segments, `**`); no relevance
  ranking; NFC/NFD unified but accents not folded; no typos.
- `doc/inner/virtual-list.md`: browsers cap an element at about 33.5 M px, so a native virtual
  list breaks past about 1.4 M rows of 24 px; Cardinal draws its own scrollbar.

### 2.9 Windows Search

- Windows 11 keeps the catalog in SQLite `Windows.db` (Windows 10: ESE `Windows.edb`) **[doc]**
  [O1], **[3p]** [O2].
- Microsoft's own sizing: a typical PC indexes under 30,000 items, a power user up to 300,000;
  problems start above 400,000; it "can index up to 1 million items" and beyond may "fail or cause
  resource problems"; a rebuild can take 24 hours **[doc]** [O1]. Five million files is five times
  the ceiling.
- Backs off when "disk or CPU use is high" and waits for idle, so on a busy disk the index stays
  incomplete without telling the user **[doc]** [O1].
- AQS: a bare word means **word-starts-with**, not substring ("ana" finds "Ana López" but not
  "Mariana"); `kind:`, `datemodified:` (today, lastweek, ranges), `size:`, `ext:`; keywords are
  localised, so a Spanish Windows needs Spanish property names **[doc]** [O3].
- Diacritic-insensitive by default, with an opt-in setting **[doc][3p]** [O4]. Semantic,
  spelling-tolerant search exists only on Copilot+ PCs with an NPU, for indexed locations
  **[doc]** [O5].

### 2.10 macOS Spotlight and `mdfind`

- Per-volume indexes in `.Spotlight-V100`, fed by FSEvents and importer plugins **[3p]** [O6].
- Query language: `kMDItemFSName == "*foo*"cd`, where `c` folds case and `d` diacritics, `w` word
  boundaries; `mdfind -onlyin DIR`, `-name`, `-live`, `-count`; no typo tolerance; `mdfind`
  results unranked **[doc]** [O7], **[3p]** [O8].
- Complaints: rebuilds that take "hours or days" and do not always help, SMB shares effectively
  unsearchable, external disks unindexed, "can't find a file that exists" and `mds_stores` CPU or
  RAM storms continuing on macOS 26 **[3p]** [O6][O9].

### 2.11 WizFile and WizTree

- WizFile 3.15, WizTree 4.34 (now also macOS/Linux), Antibody Software; free for personal use
  **[doc]** [O10][O11].
- Read the MFT as administrator; WizFile gained an opt-in non-admin mode only in 3.10 (May 2024),
  by editing `RunAsAdmin=FALSE` in its `.ini`; later releases sped up the non-admin traversal
  several times **[doc]** [O10]. No published MFT-versus-walk figure for an HDD.
- In-memory database, search-as-you-type with wildcards, regex and size/date filters, column
  sort; no ranking, no accent folding documented.

### 2.12 Others surveyed

- **Raycast** moved file-name search to its own index and uses the OS index only for content
  **[doc]** [O12].
- **UltraSearch** (JAM) reads the MFT; free for home use **[author]** [O13].
- **Listary** ranks fuzzy matches by the user's habits **[author]** [O14].
- **Find Any File** searches without Spotlight, including Synology shares **[author]** [O15].
- **HoudahSpot** is a friendly Spotlight front end with templates such as "Recent files"
  **[author]**.
- **PowerToys Command Palette / Flow Launcher** delegate to Windows Search or to an Everything
  plugin that requires Everything installed **[3p]**.

## 3. Comparison table

Name and path search only. "RAM / 1 M" is resident memory per million indexed files.

| | Enumeration | Stays current | Index form | RAM / 1 M | Disk / 1 M | Default match | Ranking | Accents | Typos | Admin | Licence |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Everything 1.4/1.5 | MFT via service; folder walk without it | USN replay; folder index: watch + daily rescan | RAM arrays, `.db` snapshot | ~100 MB [doc] | ~45 MB [doc] | substring, name only | column sort | folded by default, inconsistent in filters | no (soundex opt-in) | service install | MIT-style freeware, closed |
| WizFile | MFT; walk opt-in | FS monitoring | RAM | n/p | none | substring, regex | column sort | no | no | yes by default | free personal |
| FSearch | walk | inotify/fanotify + schedules | RAM pre-sorted arrays | ~90 MB [est] | file DB | substring, regex | column sort | one way (NFD) | no | no | GPL-2 |
| Cardinal | walk (FDA) | FSEvents resumed from saved id | RAM slab + unique-name map, zstd file | ~60–80 MB [est] | n/p | substring on unique names | column sort | NFC/NFD only | no | no (FDA) | MIT |
| Kensaku | jwalk + stat | notify; full walk at start | SQLite + RAM arena (+ Tantivy) | ~1 GB at 5 M [est] | n/p | fuzzy subsequence | RRF + bonuses | none for content | fuzzy | no | MIT |
| Windows Search | crawl + USN | backs off when busy | ESE / SQLite | n/a | ~10 % of content [doc] | word prefix | undocumented | folded by default | Copilot+ only | no | OS |
| Spotlight | FSEvents + importers | FSEvents | per-volume, undocumented | n/a | n/p | attribute + wildcards | usage (UI) | opt-in `d` | no | no | OS |
| Agent Ransack | live walk per search | n/a | none | n/a | none | substring, boolean | none | n/p | LIKE (content) | no | proprietary |
| **VFS (phase 1)** | walk, one I/O stream | own RDCW / FSEvents watcher; **full re-walk at every start** | SQLite FTS5 trigram, contentless | page caches, < 300 MB budget; 123 MB peak at 2 M [measured] | ~445 MB [est] (889 MB at 2 M [measured]) | substring, name + path | tiers: exact, prefix, name, path, typo | folded both ways | one edit, words ≥ 6 chars | no | private |

n/p: not published. Content tools (Recoll, DocFetcher, AnyTXT, rga) are in §2.4–2.6.

### What each does for a non-expert

| Tool | What helps an older, non-technical user | What hurts |
|---|---|---|
| Everything | instant list, path on every row, history, bookmarks, Spanish in 1.5 | grid chrome, regex/operators in the same box, no relevance, English-first docs |
| Windows Search | built in, nothing to install | word-prefix matching, silently incomplete, slow outside indexed places |
| Spotlight / HoudahSpot | one box; HoudahSpot templates ("Recent files") | misses files with no explanation |
| Kensaku | guided setup, removable filter chips, hints on empty results | health warnings users could not act on (issues 94, 142, 146, 176) |
| Agent Ransack | anchor-plus-offset dates as two dropdowns, saved searches | one search runs for minutes on a big archive |
| Recoll | filter panel that writes the query, spelling suggestions | technical Qt UI, hidden skip log |

## 4. Everything against VFS

The owner had not tried Everything when VFS was specified (spec decision 5: "Untried; an owned,
extensible product"). This is
the frank answer for the user's core need: find a file by part of its name or folder in a 5 M-file
archive on one busy HDD, on a Windows PC, without understanding search syntax.

### What Everything already does better

1. **First load.** About 1 minute per million files from the MFT **[doc]**, so roughly 5 minutes
   for this archive, against a full directory walk that is unmeasured on the real HDD but
   plausibly tens of minutes to hours **[est]** (issue 33 measures it).
2. **Catching up after the app was closed.** Seconds, by replaying the USN journal. VFS re-walks
   every root at every start.
3. **Breadth of results and sorting.** Everything lists every match — millions — and sorts by
   name, size or date instantly. VFS stops at 2,000 ranked hits and has no sort control, so "the
   newest file called *factura*" among 10,000 matches is out of reach unless a filter narrows it.
4. **Warm-RAM queries.** Every query scans RAM; no query waits for a cold disk. VFS's FTS pages
   live on disk and the first query after boot may read from the HDD (issue 18).
5. **Footprint.** About 0.5 GB RAM and 225 MB on disk at 5 M files **[est from doc]** against
   VFS's ~2.2 GB index at 5 M **[est, linear from 889 MB at 2 M measured]** (VFS-4, on its own
   branch, reduces it).
6. **Maturity and extras.** Twenty years of edge cases, folder sizes, duplicates, a syntax that
   covers every filter, and since April 2026 a Spanish translation.

### What VFS does that Everything cannot

1. **Typo tolerance.** "escirtura" finds "escritura"; Everything finds nothing.
2. **Relevance ranking.** Exact and prefix name matches first, then name, then path, then typos.
   Everything shows matches sorted by name, so the right file can be anywhere in a long list.
3. **Words split between folder and name.** "gestoria 2019 escritura" finds
   `Gestoría 2019\…\escritura.pdf` by default; Everything matches names only unless the user knows
   `path:` or types a backslash.
4. **Consistent accent folding** in both directions, everywhere, with no mode to toggle.
5. **No administrator rights, no service, no exposure.** Everything's speed needs a service that
   an admin installs and that gives every local user the full list of file names.
6. **A screen designed for an older reader** in plain Spanish (large type, one action per row, no
   syntax), instead of a power-user grid.
7. **Crash safety.** Every batch is committed; Everything 1.4 saves only on exit and rebuilds after
   unclean shutdowns.
8. **Throttling designed for one busy HDD** shared with Synology sync (one I/O stream, cooldowns,
   background priority) instead of Everything's NAS-oriented folder rescans that users report
   stalling other I/O.
9. **macOS**, and an owned codebase that can grow into Spanish content search (Fase 3) and the
   optional assistant (Fase 5). Everything's content indexing is RAM-resident and not recommended
   past 1 GB of text.

### Can VFS get Everything-class enumeration speed without admin?

**For the first full load: no.** Reading the MFT (`FSCTL_ENUM_USN_DATA` or a raw volume read)
fails with access denied for a standard user ([windows-io.md §4](windows-io.md)); every tool
studied either installs an elevated service (Everything, AnyTXT), asks for admin (WizFile,
WizTree, UltraSearch, Find Any File's "Find All"), or walks directories and calls that the slow
path. The walk can be made cheaper — one `NtQueryDirectoryFile` call with a large buffer per
directory (`FileIdExtdDirectoryInfo`, R10; Everything 1.5's large-fetch option is the same idea),
one I/O stream, no per-file `stat` — but it still touches every directory's index blocks on the
HDD.

**For staying current after the first load: yes, very nearly.** That is where Everything's
"instant" comes from day to day, and it does not need the MFT:

- **Windows:** `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL` works for a standard user on current builds
  (undocumented; [windows-io.md §4.1](windows-io.md)). With a saved USN and a file-reference map
  filled during the walk, a start becomes "replay what changed" instead of "re-walk 5 M files"
  (R5, issue 32). If the journal wrapped, fall back to the walk, as Everything does.
- **macOS:** FSEvents accepts a starting event id; Cardinal persists `last_event_id` and replays
  from it at launch. VFS uses `notify`, which starts from "now"; it needs its own FSEvents stream
  with `sinceWhen` (proposal P1).
- **Fallback without a journal:** skip re-listing leaf directories whose own mtime has not changed
  (R9, issue 34) — directory mtime changes on create, delete and rename of an entry, not on a file
  edit, so this misses size and date changes of existing files until a periodic full walk.

**Optional accelerator, not recommended:** if Everything is installed with its service, a VFS
process could ask it for names over the MIT IPC SDK. It would make Everything a hard dependency of
the fast path, needs an admin install once, needs Everything running in the same session (1424b),
and works on Windows only. The proposals list it under "do not do".

## 5. What the study says about the VFS design

**Confirmed by the prior art.**
- Substring matching over name **and** path (Windows Search's word-prefix default is the
  documented frustration; Everything's name-only default needs syntax to reach folders).
- Two-way accent folding applied to index and query alike (every other tool gets part of this
  wrong).
- One-typo tolerance (no file-name tool offers it; Kensaku's fuzzy subsequence is a different,
  noisier model).
- Incremental commits with the work queue in the same transaction (Everything 1.4's save-on-exit
  and Kensaku's interruption rollback are the failure modes).
- One I/O stream on the HDD (fd's own maintainers doubt gains past a few threads; WSL users found
  the default far slower).
- An unreachable root becomes unavailable and nothing is deleted (Kensaku's "a cancelled walk
  returns nothing" rule, arrived at independently).
- One exclusion list for walker and watcher, including Synology's `@eaDir` and `#recycle`.
- A watcher buffer of 64 KB and a re-walk on overflow (the same as Everything's folder indexing),
  with a cooldown (Everything's folder-indexing docs describe no cooldown, and its users
  report rescans stalling other I/O [E10]).

**Where the study disagrees.**
1. **The catch-up re-walk at every start (spec I4) is the design's biggest cost.** No fast tool
   re-walks at start: Everything and Cardinal replay a journal, and even Everything's no-admin
   folder mode rescans daily, not per launch — and that mode is still its most complained-about
   behaviour. On a 5 M-file HDD shared with Synology, each launch would start hours of seeking
   **[est]**. Issues 32 (USN) and 34 (leaf-directory skip) exist in Fase 2; the study argues they
   should be done before the first install on the real PC (issue 17), and that macOS should
   resume FSEvents from a saved id (new issue).
2. **The 2,000-hit cap with relevance as the only order.** It keeps the list fast, but it removes
   what Everything users rely on most: sorting a broad match by date or size. Any sort control
   added later must sort in SQL before the cap, never re-sort the capped list, or "más recientes"
   would silently show the newest of the 2,000 best-ranked hits rather than the newest overall.
3. **Index footprint.** At about 2.2 GB for 5 M files **[est]**, the index is five to ten times
   Everything's on-disk snapshot, on the system disk of a slow PC, and cold queries pay for it.
   VFS-4 (position-less `files_fts`, 16 KB pages, fewer pages read per query; implemented on
   its own branch) addresses this. Cardinal's unique-name
   map suggests one more lever: archives repeat names massively, and an FTS table over distinct
   folded names (with files pointing to a name id) could shrink `files_fts` by the duplicate
   ratio. Worth measuring on the real archive (new issue).
4. **No memory of the user.** Everything has history and bookmarks; Listary, Kensaku and Windows
   Search rank by what the user opened. VFS forgets every search and never learns which results
   were used, which matters more for a user with short-term memory difficulties than for a power
   user.

## Sources

### Everything (E)

- [E1] <https://www.voidtools.com/downloads/>, <https://www.voidtools.com/Changes.txt>
- [E2] 1.5 beta upgrade guide: <https://www.voidtools.com/forum/viewtopic.php?p=81898>
- [E3] 1.5 changelog thread: <https://www.voidtools.com/forum/viewtopic.php?t=9787&start=1000>
- [E4] void on MFT copy, USN, byte code: <https://www.voidtools.com/forum/viewtopic.php?p=33240>
- [E5] FAQ: <https://www.voidtools.com/faq/>
- [E6] rebuild reasons, journal size: <https://voidtools.com/forum/viewtopic.php?p=77753>
- [E7] Everything Service: <https://www.voidtools.com/support/everything/everything_service/>
- [E8] Folder indexing: <https://www.voidtools.com/support/everything/folder_indexing/>
- [E9] FindFirstFile and ReadDirectoryChanges: <https://voidtools.com/forum/viewtopic.php?p=58398>
- [E10] NAS rescan slowing the PC: <https://voidtools.com/forum/viewtopic.php?p=79831>
- [E11] Indexes and fast sort: <https://www.voidtools.com/support/everything/indexes/>
- [E12] "How is Everything so efficient?": <https://voidtools.com/forum/viewtopic.php?p=63731>
- [E13] parent-pointer compression: <https://www.voidtools.com/forum/viewtopic.php?p=43869>
- [E14] 700 MB for 8 M files: <https://www.voidtools.com/forum/viewtopic.php?p=80604>
- [E15] 100 MB per million; 30 M items: <https://voidtools.com/forum/viewtopic.php?p=31241>
- [E16] saves only on exit: <https://www.voidtools.com/forum/viewtopic.php?p=44480>
- [E17] daily auto-save in 1.5: <https://voidtools.com/forum/viewtopic.php?p=74695>
- [E18] Searching: <https://www.voidtools.com/support/everything/searching/>
- [E19] Search modifiers: <https://www.voidtools.com/support/everything/search_modifiers/>
- [E20] Search functions: <https://www.voidtools.com/support/everything/search_functions/>
- [E21] Everything 1.5: <https://www.voidtools.com/everything-1.5/>
- [E22] diacritics default: <https://www.voidtools.com/forum/viewtopic.php?p=41403>
- [E23] diacritics in filters: <https://voidtools.com/forum/viewtopic.php?p=30981>
- [E24] translating 1.5: <https://voidtools.com/forum/viewtopic.php?p=81050>
- [E25] content indexing: <https://voidtools.com/forum/viewtopic.php?p=35395>
- [E26] SDK: <https://www.voidtools.com/support/everything/sdk/>; SDK is MIT:
  <https://voidtools.com/forum/viewtopic.php?p=20947>
- [E27] SDK 3: <https://www.voidtools.com/forum/viewtopic.php?p=75613>,
  <https://github.com/voidtools/everything_sdk3>, <https://github.com/voidtools/es>
- [E28] licence: <https://www.voidtools.com/License.txt>
- [E29] redistribution: <https://www.voidtools.com/forum/viewtopic.php?p=21270>
- [E30] novice overwhelmed: <https://www.voidtools.com/forum/viewtopic.php?p=51549>
- [E31] reviews: <https://www.windowscentral.com/microsoft/windows-11/6-alternatives-to-replace-the-search-functionality-on-windows-11>,
  <https://sourceforge.net/app/everything/>

### Kensaku (K)

- [K1] <https://github.com/yjid/kensaku>, permalinks:
  `https://github.com/yjid/kensaku/blob/b8b009f45e5a06f33f877d9305f12db46b451b89/<path>`; files
  cited inline (`src-tauri/src/indexer/*`, `search/*`, `docs/ARCHITECTURE.md`,
  `docs/BENCHMARKS.md`, `tests/golden_set.rs`, `LICENSE`).

### Content tools (C)

- [C1] <https://blog.mythicsoft.com/why-regular-expressions-are-not-supported-on-index-searches/>,
  <https://blog.mythicsoft.com/version-2022-release/>
- [C2] <https://help.mythicsoft.com/agentransack/en/boolean_expressions.htm>,
  <https://help.mythicsoft.com/agentransack/en/boolean_expressions_settings.htm>
- [C3] <https://help.mythicsoft.com/agentransack/en/datetime_selection.htm>
- [C4] <https://help.mythicsoft.com/filelocatorpro/en/favorites.htm>,
  <https://help.mythicsoft.com/filelocatorpro/en/hintstips.htm>
- [C5] <https://www.mythicsoft.com/agentransack/download/>
- [C6] <https://redlib.cypherpunktools.com/r/DataHoarder/comments/1jrwex1/using_agent_ransack_is_there_something_better/mlnphsq/?context=3>
- [C7] <https://www.recoll.org/pages/download.html>, <https://www.recoll.org/pages/recoll-windows.html>
- [C8] <https://www.recoll.org/usermanual/usermanual.html>
- [C9] <https://man.archlinux.org/man/recollindex.1.en>
- [C10] <https://framagit.org/medoc92/recoll/-/issues/93>, <https://framagit.org/medoc92/recoll/-/issues/24>,
  <https://bugs.launchpad.net/ubuntu/+source/recoll/+bug/718427>
- [C11] <https://docfetcher.sourceforge.io/>, <https://sourceforge.net/p/docfetcher/wiki/FAQ>
- [C12] <https://docfetcherpro.com/changelog-docfetcher-pro/>
- [C13] <https://sourceforge.net/p/docfetcher/bugs/1367/>
- [C14] <https://anytxt.net/>, <https://sourceforge.net/projects/anytxt/>,
  <https://www.exploit-db.com/raw/49549>, <https://alternativeto.net/software/anytxt-searcher/about>
- [C15] <https://github.com/phiresky/ripgrep-all>,
  <https://github.com/phiresky/ripgrep-all/blob/master/src/preproc_cache.rs>
- [C16] <https://github.com/sharkdp/fd>, <https://github.com/sharkdp/fd/blob/master/src/walk.rs>
- [C17] <https://github.com/sharkdp/fd/issues/1131>

### OS indexers and newer tools (O)

- [O1] <https://learn.microsoft.com/en-us/troubleshoot/windows-client/shell-experience/windows-search-performance-issues>
- [O2] <https://levelblue.com/blogs/strozfriedberg/windows-search-index-the-forensic-artifact-youve-been-searching-for>
- [O3] <https://learn.microsoft.com/en-us/windows/win32/search/-search-3x-advancedquerysyntax>
- [O4] <https://learn.microsoft.com/en-us/windows/win32/search/-search-sql-accentinsensitivitysearches>,
  <https://www.elevenforum.com/t/enable-or-disable-use-of-diacritics-in-search-index-in-windows-11.48559/>
- [O5] <https://support.microsoft.com/en-us/windows/search-indexing-in-windows-da061c83-af6b-095c-0f7a-4dfecda4d15a>,
  <https://www.microsoft.com/en-us/windows/learning-center/find-files-fast-with-improved-search>
- [O6] <https://eclecticlight.co/2024/11/29/using-and-troubleshooting-spotlight-in-sequoia-summary/>
- [O7] <https://developer.apple.com/library/archive/documentation/Carbon/Conceptual/SpotlightQuery/Concepts/QueryFormat.html>
- [O8] <https://eclecticlight.co/2025/06/06/how-to-search-successfully-in-spotlight-query-languages/>
- [O9] <https://discussions.apple.com/thread/256140958>, <https://developer.apple.com/forums/thread/796568>
- [O10] <https://antibody-software.com/wizfile/download>, <https://antibody-software.com/wizfile/>
- [O11] <https://diskanalyzer.com/whats-new>
- [O12] <https://manual.raycast.com/file-search>
- [O13] <https://www.jam-software.com/ultrasearch>
- [O14] <https://www.listary.com/>
- [O15] <https://apps.tempel.org/FindAnyFile/>
- [O16] <https://github.com/cboxdoerfer/fsearch> (`src/fsearch_database_entry.c`,
  `fsearch_database_index_store.c`, `fsearch_array.c`, `fsearch_utf.c`)
- [O17] <https://github.com/cardisoft/cardinal> (`namepool/`, `search-cache/`,
  `doc/inner/search-cache.md`, `doc/inner/virtual-list.md`, `CHANGELOG.md`)
