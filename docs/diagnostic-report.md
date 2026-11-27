# Reading a VFS diagnostic report

"Enviar comentarios" (VFS-20) saves `vfs-informe-<AAAA-MM-DD-HHMM>.zip` in the user's
Downloads folder. It holds one file, `informe.txt`, the whole report: the text the dialog's
"Ver el informe" shows, which carries only the newest ~200 KB of log lines (VFS-30), with
every log line. This page says what is in it and how to analyse it in a session on this repo.

## Getting a report

- **From a user**: they press "Enviar comentarios", write what happened and press "Enviar
  por GitHub" (a prefilled issue from the `comentarios` form, labelled `feedback-usuario`; they drag the `.zip` in) or
  "Exportar diagnóstico (.zip)" and hand the file over.
- **For your own analysis of a PC after weeks of use**: on that PC open "Enviar comentarios",
  leave the text empty, and press "Exportar diagnóstico (.zip)". The report never holds folder
  or file names (VFS-48); for a "cannot list" or watcher problem that needs them, read the raw
  log on that PC with the owner.
- The raw log, if you need it without the app: `%LOCALAPPDATA%\app.vfs.desktop\logs\` on
  Windows, `~/Library/Application Support/app.vfs.desktop/logs/` on macOS (`vfs.log` and up to
  ten rotated `vfs_<date>.log`, 22 MB at most). The raw files are not redacted.

## What the report holds

| Section | Content |
|---|---|
| `## What happened` | The user's own words, with every path as `<ruta>` and the account name as `<usuario>` (VFS-48). |
| `## App` | Version, OS (`windows 10.0.<build> <release>`; a build ≥ 22000 is Windows 11), local time it was made. |
| `## Folders` | One line per root: id, `drive` (local/network/removable), `disk` (ssd/hdd/unknown), status, files, folders, `cloud_only` and `links` (at the last full walk, `?` before one), `unreadable` and `unpopulated` folders (cloud placeholders that listed empty, Windows; VFS-37), time since the last full walk. Counts carry no name, so they go into the summary too. The path never appears (VFS-48). |
| `## Index` | Files, folders, roots, index size, WAL size (MB), schema version. |
| `## Settings` | Indexer state (idle/scanning/paused); `bookmarks: on (browsers=firefox,chrome profiles=… bookmarks=…)`, `off` or `unavailable` (VFS-44). |
| `## Log` | Number of log files and lines; warnings, errors and panics in them. |
| `## Log lines` | Every kept log file, oldest first, each under `--- <file name> ---`. |

Log lines are `[date][time UTC][target][LEVEL] message`. Targets worth knowing:

- `vfs::diag` (INFO): the indexer's, the index open's and the root changes' records (VFS-5 R7,
  VFS-31 R5, VFS-41),
  never redacted because they hold only numbers. `start` at each launch (uptime, deferred catch-up, disk-load probe, index folder in a
  sync root); `pass` every minute of a walk and `pass_end` per walk (mode, dirs and files per
  minute, `backoff_s` spent waiting for a busy disk, commit and checkpoint ms, retries,
  errors; `pass_end` also the process's `read_mb` / `written_mb` during the walk); `memory`
  every 10 min (working set, WAL and index MB, then since launch: `peak_mb` peak memory,
  `sqlite_mb` / `sqlite_peak_mb` SQLite's heap now and at its peak, `cpu_s` CPU seconds,
  `read_mb` / `written_mb`; idle CPU is the `cpu_s` difference between two `memory` lines over
  the minutes between them); `open` at each index open (`ms`, `checked=1` when the integrity
  check ran after an unclean exit, `recovered=1` when the index was rebuilt, `index_mb`);
  `compact` once per index compaction after large deletions (deletions merged, slices, busy
  ms, vacuum done/skipped, index MB before->after; VFS-40); `watch_overflow` when a
  watcher lost events and a re-walk was queued; `root_add` (root id, roots, folders and files
  taken in, ms), `root_remove` (root id, ms) and `root_removed` when the indexer has deleted
  a removed root's rows (root id, folders, files, ms).
- `vfs::panic` (ERROR): a panic, with the source location in this repo. Only
  `panic at <file>:<line>: <omitido>` is kept: the payload is free text and can name files.
- Any other target (WARN/ERROR): problems such as unreadable folders or watcher failures.
  Every path in them reads `<ruta>` and the account name `<usuario>`, and the redaction may swallow the rest of
  the clause (`root <ruta>` for "root … became unreachable"); after a macOS path (`/…`, `~/…`)
  it swallows the rest of the line, error text included, since a macOS name can hold `": "`.
  A continuation line (no `[date]…` prefix) reads `<omitido>` unless it follows a `vfs::diag`
  record.

## Analysing it in a Claude session

Unzip the report outside the repo (it is user data; never commit it) and start a session on
this repo with a prompt like:

> Read docs/diagnostic-report.md, then analyse the VFS report at `<path>/informe.txt`. From
> the `vfs::diag` records, chart walk rate, back-off and commit/checkpoint times over the
> weeks, list each warning and error kind with counts and first/last time, explain any
> panic from its location, and compare with the thresholds in
> specs/vfs-5-windows-tuning/spec.md. Propose changes ranked by expected gain on this PC.

Useful greps on `informe.txt`: `grep -c 'pass_end'` (walks), `grep 'backoff_s=[1-9]'`
(time lost to a busy disk), `grep -E '\[(WARN|ERROR)\]' | cut -d']' -f3- | sort | uniq -c`
(warning kinds), `grep vfs::panic`.
