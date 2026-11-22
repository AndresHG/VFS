# VFS-31 — Removing a folder finishes on screen; adding its parent takes it in

- **Status**: approved after an independent critic review (its changes folded in)
- **Size**: S+ (a bug with a repro, plus one owner product decision on the same flow; no
  schema change)
- **Links**: issue 154; VFS-1 AC-4, U4 (overlapping roots), I6 (removal in batches); VFS-5
  R7, R8 and VFS-30 (diagnostic records carry no names)

## Problem

On v0.2.0 (macOS) the owner had one folder indexed (31,000 files, 971 folders) and tried to
add a folder containing it. The app refused and told them to remove the inner folder first.
After "Quitar carpeta" the row stayed at "Quitando…" for good; after a restart the folder
was gone. The report showed `roots: 0` and no log line about either change.

Cause: `remove_root` only marks the root `removing` and wakes the indexer, which deletes
its rows (`finish_removing`) and then reports idle. The indexer sends an idle snapshot only
when its state changes, and it was already idle before the removal, so no progress event
followed. The UI re-reads the folder list only on a progress event, so it never saw the root
go. The cause is in the core, so Windows is affected the same way.

## Requirements (EARS)

- R1. WHEN the root list changes (a root added or removed), THE indexer SHALL send a progress
  snapshot once it has handled the change, even if its state (idle) is the same as before
  (`RootsChanged` forgets the last published state), so the UI re-reads the folder list and
  the removed folder leaves it. WHILE the indexer is paused, the removal and its snapshot
  wait for the resume (the row reads "Quitando…" meanwhile). THE UI SHALL apply only the
  newest folder list it asked for, so an older reply cannot bring a removed folder back.
- R2. WHEN the user adds a folder that contains one or more listed roots (not being
  removed), THE core SHALL accept it: the new root is inserted first, then takes in each
  inner root's folders and files, with their ids, in the same transaction: the folders
  between the new root and the inner one are created pending deep, each moved folder's
  path, `dirs_fts` row and its files' `file_paths_fts` rows are rewritten (the latter
  deleted by their old value first, schema v3), the inner root's own folder hangs from its
  new parent, the inner root rows and change cursors are deleted, and the new root starts
  with the taken-in counts. The new root is then walked as any new root. The inner files
  stay searchable throughout and are not deleted and inserted again.
- R2a. WHEN the indexer applies a listing, THE indexer SHALL first check, inside the write
  transaction, that the folder still belongs to the root and path it read before listing,
  and otherwise drop the listing and re-read the roots, so a take-in committed during a
  listing never lands rows under the old root.
- R3. WHEN a folder is added that is the same as or inside a listed root, THE core SHALL
  still refuse it (`overlapping_root`, relation `same` or `inside`); the UI says it is
  already included, as before.
- R4. WHEN an added folder took in listed folders (R2), THE UI SHALL say so in one short
  notice naming them, in Spanish and English (`strings.es.js`, `strings.en.js`).
- R5. THE core SHALL write a diagnostic record (`vfs::diag`) for each root added (root id,
  roots taken in, folders and files taken in, milliseconds), each root marked for removal
  (root id, milliseconds) and each removal finished by the indexer (root id, folders, files,
  milliseconds). The records hold only ids, counts and durations: no path, name or search
  text. The facade writes its records straight to the `vfs::diag` log target, as the
  indexer's default sink does.

## Acceptance scenarios

Tests cite these ids (`VFS-31 AC-<m>`) in their doc comments.

- AC-1 (R1) Given a running indexer that has scanned a root and reports idle, When the root is
  removed, Then a progress event arrives with no roots, and the root list is empty.
- AC-2 (R2) Given an indexed root `Datos/Fotos` with files, When `Datos` is added, Then the
  list holds only `Datos`; the files of `Fotos` keep their ids and are found by name and by
  path (`fotos verano`) under `Datos`, before any walk; after the walk the index matches the
  disk with no duplicate folder rows. Two inner roots sharing an intermediate folder are
  taken in together.
- AC-3 (R3) Same folder and a folder inside a root are still refused with `same` / `inside`.
- AC-4 (R4) Folders being removed are left out of the notice. Both catalogs carry the notice with the same keys; the old "remove it first"
  text is gone.
- AC-5 (R5) The add, remove and removal-finished records are `key=value` pairs of numbers
  only.

## Non-goals

- Splitting a root into its subfolders (removing a parent and keeping a child).
- A progress bar for removal: the row already reads "Quitando…" until R1's event.

## Pre-mortem

- The move rewrites every taken-in file's path in the path index inside one transaction; on
  a million files that holds the write lock for seconds. Accepted: it is a one-off and still
  far cheaper than a disk walk plus re-insertion; the add command runs off the UI thread.
- A child folder row created by the walk for the inner root's folder before the move would
  collide on `UNIQUE(root_id, rel_path)`. Avoided: the move creates the intermediate folders
  itself and links the inner root folder to them, so the walk finds it as an existing child.
- A taken-in folder whose stored name differs in bytes from the disk (case, NFC/NFD) is
  deleted and inserted again by the walk, which matches children by exact name. Accepted:
  the index stays correct, only the saving is lost for that folder.
- Paths compared by folded keys could differ in case from the stored spelling; the relative
  part is taken from the inner root's own stored path by component count, not by string
  prefix.

## Decisions

- D1 (owner, issue 154): a containing folder is accepted with a notice instead of asking to
  remove the inner one first.
- D2: the UI works out which roots were taken in from the list it already holds (as the
  overlap message did), so the `Root` type does not change.
