# Reading a VFS diagnostic report

"Enviar comentarios" (VFS-20) saves `vfs-informe-<AAAA-MM-DD-HHMM>.zip` in the user's
Downloads folder. It holds one file, `informe.txt`, exactly the text the dialog's "Ver el
informe" shows. This page says what is in it and how to analyse it in a session on this repo.

## Getting a report

- **From a user**: they press "Enviar comentarios", write what happened and press "Enviar
  por GitHub" (a prefilled issue with the `comentarios` label; they drag the `.zip` in) or
  "Exportar diagnóstico (.zip)" and hand the file over.
- **For your own analysis of a PC after weeks of use**: on that PC open "Enviar comentarios",
  leave the text empty, and press "Exportar diagnóstico (.zip)". Tick "Incluir en el informe
  los nombres de carpetas y archivos" only with the owner's consent; it adds the root paths and
  keeps folder names in warnings, which helps with "cannot list" and watcher problems.
- The raw log, if you need it without the app: `%LOCALAPPDATA%\app.vfs.desktop\logs\` on
  Windows, `~/Library/Application Support/app.vfs.desktop/logs/` on macOS (`vfs.log` and up to
  ten rotated `vfs_<date>.log`, 22 MB at most). The raw files are not redacted.

## What the report holds

| Section | Content |
|---|---|
| `## What happened` | The user's own words. |
| `## App` | Version, OS (`windows 10.0.<build> <release>`; a build ≥ 22000 is Windows 11), local time it was made, whether names are included. |
| `## Folders` | One line per root: id, `drive` (local/network/removable), `disk` (ssd/hdd/unknown), status, files, folders, time since the last full walk. The path only with names included. |
| `## Index` | Files, folders, roots, index size, WAL size (MB), schema version. |
| `## Settings` | Indexer state (idle/scanning/paused), Firefox bookmarks on/off with profiles and count. |
| `## Log` | Number of log files and lines; warnings, errors and panics in them. |
| `## Log lines` | Every kept log file, oldest first, each under `--- <file name> ---`. |

Log lines are `[date][time UTC][target][LEVEL] message`. Targets worth knowing:

- `vfs::diag` (INFO): the indexer's records (VFS-5 R7), never redacted because they hold only
  numbers. `start` at each launch (uptime, deferred catch-up, disk-load probe, index folder in a
  sync root); `pass` every minute of a walk and `pass_end` per walk (mode, dirs and files per
  minute, `backoff_s` spent waiting for a busy disk, commit and checkpoint ms, retries,
  errors); `memory` every 10 min (working set, WAL and index MB); `watch_overflow` when a
  watcher lost events and a re-walk was queued.
- `vfs::panic` (ERROR): a panic, with the source location in this repo.
- Any other target (WARN/ERROR): problems such as unreadable folders or watcher failures.
  Without names, every path in them reads `<ruta>`, and the redaction may swallow the rest of
  the clause (`root <ruta>` for "root … became unreachable").

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
