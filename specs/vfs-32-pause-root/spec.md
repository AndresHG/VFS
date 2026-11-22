# VFS-32 — Pause and resume one folder while it is indexed

- **Status**: approved after an independent critic review (approve-with-changes, applied)
- **Size**: M (indexer root selection, one command, the folder row; no schema change: the
  pause is a `meta` key, like VFS-9's cursor)
- **Links**: issue 96 (id); VFS-1 I10 (one full walk at a time), VFS-9 (change cursor),
  VFS-26 (history replay), VFS-31 (same folder row)

## Problem

Adding a huge folder by mistake (a whole home folder, 3 M files) starts a first walk of
hours. Today the row offers "Volver a revisar", which does nothing while the walk runs, and
"Quitar", which throws away what was indexed. The status-line "Pausar" stops every folder.
The user needs to stop one folder, keep searching what is indexed, keep the others up to date
and continue later.

## Requirements (EARS)

- R1. THE core SHALL offer `set_root_paused(root_id, paused)`, stored as the `meta` key
  `paused/<root id>` (present = paused), so a pause survives a restart until resumed.
  `NotFound` for an unknown root or one being removed. Deleting a root's rows deletes the
  key with its cursor, so a reused root id is never born paused.
- R2. WHILE a root is paused, THE indexer SHALL list none of its pending folders, start no
  walk of it (an owed walk stays owed), not watch it (the watch is dropped, which frees its
  buffers) and not replay its history at start; its pending folders stay pending in `dirs`,
  so the walk keeps its place. A root paused mid-walk SHALL be left within one folder (the
  current batch commits first). No query that schedules work SHALL count its rows: the
  pass mode (VFS-5 R1) ignores its pending folders, the shortcut pass (VFS-18 D7) its links,
  and watch overflows or failures queued before its watch was dropped are ignored.
- R3. A paused root SHALL NOT count as being walked: other roots keep indexing, and their
  owed walks start (VFS-1 I10's one-walk rule ignores paused roots).
- R4. WHILE a root is paused, its stored cursor SHALL stay where it is (VFS-9 R6: an
  unwatched root's cursor does not advance); every walk stores one when it starts, so the
  resume can replay what changed meanwhile.
- R5. WHEN a root is resumed, THE indexer SHALL watch it again, replay its history from the
  stored cursor and mark the changed folders pending (as VFS-26 R3), then continue its
  pending folders where the walk stopped. IF the replay cannot run (no history, journal
  wrapped), THEN a root being walked SHALL restart its walk in place (root folder pending
  deep, cursor of now stored), one walk instead of the rest plus a deferred second one; any
  other root SHALL hold the cursor and owe the deferred lost-events walk (as VFS-26 R4).
- R6. Searches SHALL keep finding what a paused root already indexed.
- R7. Pause and resume SHALL each write one diagnostic line with the root id only:
  `root_pause root=<id>` and
  `root_resume root=<id> folders=<n> result=replayed|rewalk|lost`.
- R8. `Root` SHALL carry `paused`. The folder row's first button SHALL read "Pausar" while
  the root is pending or scanning and not paused, "Reanudar" while paused (the state text
  reads "En pausa"), and "Volver a revisar" otherwise (enabled only when ready or
  unavailable, as before). "Quitar" is unchanged. Copy in both catalogs.

## Acceptance scenarios

- AC-1 (R2, R5) Given a root mid first walk, When it is paused, Then no further folder of it
  is listed while paused; When resumed, Then its walk finishes and it is ready.
- AC-2 (R1) Given a paused root, When the index is closed and reopened, Then it is still
  paused and its walk does not advance until resumed.
- AC-3 (R3) Given root A paused mid walk and root B added, Then B's first walk completes
  while A stays paused.
- AC-4 (R1) Removing a paused root deletes its pause key; `set_root_paused` on an unknown id
  is `NotFound`.
- AC-5 (R6) A file indexed before the pause is found while paused.
- AC-6 (R8) The row's button label for pending, scanning, paused, ready and unavailable.
- AC-7 (R2, R5, R7) A running indexer does not replay a paused root at start, ignores its
  queued overflow and failure, writes both diagnostic lines, and the resume replays.
- AC-8 (R2) With A paused and more than 64 folders pending, the pass mode is incremental.
- AC-9 (R5) A resume that cannot replay restarts a walking root's walk in place; a ready
  root is not re-walked that way.

## Decisions

- No new `RootStatus`: the status stays what the walk was (pending or scanning), so resuming
  needs no memory of which walk it was; `paused` is a separate flag.
- Resume continues from the persistent queue (`dirs.scan_state`); nothing restarts.
- Pausing a root that is not walking is allowed (a click racing the end of the walk); it
  then simply shows "Reanudar", and its owed walks wait until it is resumed.
- After a resume two walks may coexist (B started while A was paused); `run` lists them one
  after the other in root-id order, and both finish.
- A resume during the global pause runs its replay at once (one journal read); the walk
  itself waits for the global pause.
- The status line does not mention paused folders; the row says "En pausa". `paused` is on
  `Root`, so the row refreshes the root list after the click.
- `scan_to_completion` (CLI) skips paused roots too.

## Out of scope

The status-line global pause (unchanged), pausing bookmark reading, the review of the 3 M
file run (issue 156).
