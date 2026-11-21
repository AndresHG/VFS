# VFS-25 — Cloud-only files: record them, never open them

- **Status**: approved (independent architect review: two final fixes applied, nothing else blocking)
- **Size**: M (one subsystem, Windows-only behavior, no schema change: a bit of the existing
  `files.flags`)
- **Links**: issue 49; R11 in `docs/research/windows-recommendations.md`; §3.4 of
  `docs/research/windows-io.md`; VFS-18 (the link pass opens `.lnk`/`.url` files); VFS-2 and
  issue 23 (content extraction must skip these files); VFS-10 (the reason "solo en la nube")

## Problem

With Synology Drive's on-demand sync (or OneDrive), many files on the target PC are
placeholders: the name and size are local, the data stays on the NAS. Reading their data
downloads them, and Windows shows a toast. Listing them does not (VFS-1 never reads file
data), but VFS keeps nothing that says a file is cloud-only: the user cannot tell why a
result opens slowly, and two readers that do open files can download them by surprise: the
VFS-18 link pass, which reads every `.lnk` and `.url`, and the content extraction of Fase 3.

## Requirements (EARS)

- R1. WHEN a folder is listed on Windows, THE walker SHALL mark a file cloud-only when its
  enumeration attributes (`MetadataExt::file_attributes` of the `DirEntry`'s metadata, which
  comes from the listing: no extra call, no open) carry `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`
  (0x400000), `FILE_ATTRIBUTE_RECALL_ON_OPEN` (0x40000) or `FILE_ATTRIBUTE_OFFLINE` (0x1000),
  and store it as `files.flags` bit 2 (`ingest::FLAG_CLOUD_ONLY = 4`, beside `FLAG_LINK`).
  Symbolic links and junctions (VFS-18) are never marked.
- R2. WHENEVER a folder is listed, THE indexer SHALL leave every file's stored cloud-only
  bit equal to what that listing saw, whatever else changed. A folder is listed again on a
  watcher hint (a size, write-time, name or creation change), on the change-history replay at
  start (VFS-9; the journal records attribute changes) and on every walk. Attribute-only
  changes are not watched live (`FILE_NOTIFY_CHANGE_ATTRIBUTES` is left out: Synology's
  constant attribute updates would re-list folders on the HDD), so a download or a "free up
  space" made while the app runs shows at the next listing of that folder.
- R3. THE link pass SHALL never open a cloud-only link file: it gets no target and is marked
  unreadable; WHEN its cloud-only bit clears (R2), its link row SHALL become pending again, so
  the next link pass reads it.
- R4. WHEN a search returns a file, its hit SHALL carry `cloudOnly: true` when that file is
  cloud-only, and each of its VFS-19 copies SHALL carry its own `cloudOnly`. The results row
  SHALL show the badge "Solo en la nube" (English: "Cloud only") for the file it shows; the
  copies list SHALL show it on each place that is cloud-only.
- R5. THE flag SHALL be readable by content extraction through `ingest::FLAG_CLOUD_ONLY`,
  which SHALL skip such files (contract for VFS-2; nothing extracts today). No VFS code path
  outside the link pass and the user's own actions on a result reads a file's data;
  "Mostrar en carpeta" asks the shell to select the item, which does not download it.

## Acceptance scenarios

Tests cite these ids (`VFS-25 AC-<m>`) in their doc comments.

- AC-1 (R1) Unit test: attribute words with each of the three bits are cloud-only; plain,
  hidden, archive, read-only and reparse-point-only words are not; `FLAG_CLOUD_ONLY` is
  bit 2 and distinct from the lossy and link bits.
- AC-2 (R1, R2, R4; Windows CI) Given a file marked `FILE_ATTRIBUTE_OFFLINE` (the one of the
  three a program may set) and an ordinary file, When the folder is indexed and searched,
  Then only the first hit has `cloudOnly`; When the attribute is cleared without changing
  size or time and the folder is listed again (a watcher-style hint), Then the flag is gone;
  When it is set again together with a size change, Then the flag is back.
- AC-3 (R3; every platform, the bit set in the index) Given a cloud-only `.url` holding a
  valid address, When the link pass runs, Then it has no target and is unreadable (had it
  been opened, the address would be its target); When its bit clears through a listing,
  Then its link row is pending and the next pass reads the address.
- AC-4 (R4) Search test: a cloud-only file and a local copy of it (same name and size)
  group as one hit; the hit and the copy each carry their own `cloudOnly`. UI tests in
  `apps/desktop/ui-dev/screen.test.mjs`: the badge text exists in both catalogs; `placesOf`
  carries each place's `cloudOnly`; a pure helper decides the row and place badges, shown
  only when `cloudOnly` is true.

## Non-goals

- macOS iCloud "dataless" files: not detected in this change (the target PC is Windows).
- Folders that are cloud-only: listed as today (enumeration does not download data);
  a listing that waits for the provider is VFS-1 I11's business and, if it fails, VFS-10's.
- No "download now" or "free up space" action; VFS never changes a file.
- No filter by cloud-only state.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Where is the state kept? | `files.flags` bit 2 (bit 0 lossy name, bit 1 link): no schema change, no migration | Claude | 2026-11-21 |
| 2 | Which attributes? | The three of R11; `OFFLINE` included because Windows treats it as "data not immediately available" and it is the one a test can set | Claude (windows-io §3.4) | 2026-11-21 |
| 3 | Badge text | "Solo en la nube" / "Cloud only", as issue 49 proposes | owner (issue 49) | 2026-11-21 |
| 4 | Grouped copies (VFS-19) | The flag travels per place: the row's badge is for the file it shows, the copies list badges each cloud-only place | Claude (architect review) | 2026-11-21 |
| 5 | Watch attribute changes live? | No: R2 relies on listings that already happen; Synology's attribute churn would re-list folders on the HDD | Claude (architect review) | 2026-11-21 |

## Open questions

None blocking. Release check on the target PC (owner, before v0.2.0 ships to them): one
Synology on-demand file must show "Solo en la nube" (R1 assumes Synology sets one of the
three attributes; windows-io §3.4 says its use of the Cloud Files API is implied, not stated).
