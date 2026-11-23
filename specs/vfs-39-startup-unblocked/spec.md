# VFS-39 — The window shows and closes at once on a huge index

- **Status**: implemented (PR pending)
- **Size**: M (desktop shell start-up and the indexer's stop; no schema change)
- **Links**: issue 178; VFS-33 R8 (re-walk after an exclusion change); VFS-1 AC-6 (integrity
  check after an unclean exit); VFS-21 R8 (index stopped before an update); issue 108 (window
  shown on page load)

## Problem

On macOS, after the update to 0.2.2, VFS ran (about 60 % CPU) but showed no window. When the
window appeared, it seemed frozen, and closing it did not respond. The root was a home folder
of about 5 M files, and the one-off VFS-33 re-walk was dropping the newly excluded folders.
There are two causes.
1. The shell opened the index in Tauri's `setup`, on the main thread, before the event loop
   runs. After any unclean exit (a forced quit, a crash) that open runs `PRAGMA quick_check`
   over the whole index: 43 s for a 1.8 GB synthetic 5 M-file index on a fast SSD, and
   minutes on the target PC's HDD. Until it ends, no page loads, so the window never shows.
   The only fallback was showing it blank after 120 s. A forced quit then causes the next
   unclean open.
2. Closing the window (and installing an update) stops the indexer and waits for its thread.
   Some indexer steps never check the stop flag. One is the walk deleting a whole subtree that
   a new exclusion dropped, in one transaction. Deleting 1 M indexed files that way holds a
   stop for 6.5 s in a debug build on an SSD (AC-3). The owner's root lost about 3.6 M files,
   on a far larger index, so a close could wait for minutes.

## Requirements (EARS)

- R1. WHEN the app starts, THE shell SHALL open the index (integrity check, migrations,
  indexer start, bookmarks) on its own thread. The main thread SHALL go on to the event loop
  at once, so the window loads its page, shows and answers the OS whatever the open takes. A
  failed open still shows the startup error dialog (from the main thread) and quits.
- R2. WHILE the index is opening, THE index commands SHALL reject with `unavailable` and
  never hang. THE `wait_core` command SHALL resolve once the index is open, or reject with
  `unavailable` when it could not open; it waits on a blocking thread, not an async worker.
  THE UI SHALL show «Preparando la búsqueda…» / «Getting search ready…» in the status line
  and call `wait_core` before its first index command. It SHALL check for updates without
  waiting for the index.
- R3. WHEN the indexer is asked to stop (window closed, app exiting, update installing) and
  its thread has not ended within 500 ms, THE indexer SHALL interrupt its SQLite work every
  20 ms until the thread ends. The interrupted transaction rolls back (cheap in WAL mode), so
  the index stays consistent and the next walk redoes that work.

## Acceptance scenarios

Tests cite these ids (`VFS-39 AC-<m>`) in their doc comments.

- AC-1 (R1) Given an index whose open is held up (another connection holds the write lock),
  When `open_core` is called, Then it returns within 500 ms.
- AC-2 (R2) Given that held-up open, When the UI calls `list_roots`, Then it rejects with
  `unavailable`. When it calls `wait_core`, Then the call resolves only after the open is
  released, and `list_roots` then answers.
- AC-3 (R3) Given a scanned root with a folder of 1,000,000 indexed files that has left the
  disk, When a re-walk is deleting that folder's subtree and a stop is requested, Then the
  stop returns within 2 s (it took 6.5 s without R3). When the next scan runs, Then the
  folder's files are gone and the other folders' files remain.

## Non-goals

- Making the integrity check itself faster or skipping it: it still runs after an unclean
  exit, now off the main thread.
- Splitting the subtree deletion into small transactions: interrupting the step on a stop is
  enough for the close. A step that runs while the app stays open still holds the indexer
  (searches are unaffected: they read their own WAL snapshot).
- The 60 s wait for the macOS change history at start-up, which is not SQLite work and is
  bounded.

## Pre-mortem

- Data loss: none. An interrupted transaction rolls back whole; nothing the user owns is
  written.
- A stop that interrupts a normal commit: the batch rolls back and is listed again next time.
- An exit while the index is still opening: there is no indexer to stop yet. The open
  thread ends with the process, and the next open runs the integrity check again. The same
  holds for an update installed while the index is opening: the relaunch replaces the
  process.
- A search typed while the index opens waits for it and runs once it is open; a panic on
  the open thread rejects `wait_core` and shows the startup error.
- Windows and macOS: the same code on both; SQLite's interrupt is portable.
- Copy: the UI never says «índice» (existing test); the new status line avoids it.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Interrupt the long step or split it into batches? | Interrupt on a stop: smallest change, consistent by construction (rollback) | session, standing authorization (CLAUDE.md) | 2026-11-22 |
| D2 | What the UI shows while the index opens | The existing status line, reworded «Preparando la búsqueda…» | session | 2026-11-22 |

## Open questions

None.
