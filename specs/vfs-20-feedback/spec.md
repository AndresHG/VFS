# VFS-20 — "Enviar comentarios" with a diagnostic report, sent by the user

- **Status**: implemented (PR #130)
- **Size**: M (one desktop feature: a dialog, three commands, a report builder; log retention
  and a panic hook in the shell; no index schema change, nothing sent automatically)
- **Links**: VFS-5 (R7 diagnostic log, R8 no names in it), VFS-1 C7 (log file), VFS-22
  (string catalog), issue #96

## Problem

VFS runs on other people's PCs, many of them non-technical, fully offline and with no
telemetry. When something goes wrong the user cannot tell us what the program saw, and the
maintainer cannot see how the indexer behaved over weeks on a slow PC (a spinning disk, a
busy sync client). The program already writes a diagnostic log (VFS-5 R7: counts and
durations), but nobody can find it, it holds about two weeks at most, warnings in it can
name folders, and a panic leaves no trace.

We need one visible entry, "Enviar comentarios", where anyone can say what happened, see
exactly what the attached report contains, and send it themselves: through GitHub when they
have an account, or as a saved `.zip` they hand over otherwise. The same report, exported,
is what the maintainer brings to an analysis session.

## Prior art (researched 2026-11-21)

| Product | How a user reports | What we take |
|---|---|---|
| VS Code, Help → Report Issue | In-app form; system info gathered locally and shown before sending; "Preview on GitHub" opens a prefilled new-issue page; when the data is too long for the URL it goes to the clipboard and the user pastes it ([vscode#127225](https://www.github.com/microsoft/vscode/issues/127225)). Needs a GitHub account. | Prefilled GitHub issue; the user sees the data first; a URL has a length limit, so the bulk travels as an attachment. |
| Firefox, `about:support` | A local page of troubleshooting data with "copy to clipboard"; the user pastes it where they ask for help. Nothing is sent by the browser. | User-initiated only; plain text anyone can read. |
| Obsidian, "Show debug info" | Copies a short block (version, OS, settings, plugins) to paste in the forum. | A short summary that fits in a post. |
| Syncthing, support bundle | A `.zip` of config, usage report and logs; credentials redacted automatically, and the docs ask the user to check the bundle for personal data before sending ([docs](https://docs.syncthing.net/rest/debug.html)). | A single `.zip` attachment; automatic redaction plus a human check. |
| Signal Desktop, View → Debug Log | The user examines (and may edit) the log, then submits it ([support](https://support.signal.org/hc/articles/360007318591)). | Show the exact text that will be attached. |
| Everything (voidtools) | No built-in reporter; users start a debug log and post it to the forum. | Counter-example: a log nobody can find is not a report. |

Best practice we adopt: (1) nothing leaves the PC unless the user acts; (2) the report is
plain text the user can read in full before it goes anywhere; (3) private data is redacted
by default and included only by an explicit, unticked-by-default choice; (4) a short
summary prefilled into a GitHub issue form plus a `.zip` attachment for the full log;
(5) a no-account path that only saves the file.

## Requirements (EARS)

- R1. THE app SHALL show an "Enviar comentarios" button in the bottom bar, which opens a
  dialog with: a free-text box ("¿Qué ha pasado o qué le gustaría?"), an unticked box to
  include folder and file names, a "Ver el informe" disclosure showing the exact report text,
  and two actions: "Enviar por GitHub" and "Exportar diagnóstico (.zip)"; a line says the
  GitHub page is public and that folder names go only in the `.zip`.
- R2. THE report SHALL be plain UTF-8 text holding: the user's text; app version; OS name,
  version and architecture; per root folder its number, drive kind (local, network,
  removable), disk kind (SSD, HDD, unknown), status, file and folder counts and time since
  its last full scan; index stats (files, folders, roots, size, WAL size, schema version);
  settings (indexer paused, Firefox bookmarks on or off, profiles and bookmarks read); counts
  of warnings, errors and panics in the log; and every kept log file, oldest first.
- R3. WHILE the names box is unticked, THE report SHALL contain no root path, no file or
  folder name and no path: diagnostic-target lines are kept as written (VFS-5 R8 guarantees
  them free of names), and in every other log line each root path, the user's home folder
  and anything that reads as a path is replaced with `<ruta>`. No search text is ever
  recorded, so none can be in the report.
- R4. WHEN the names box is ticked, THE report SHALL also list each root's path and keep the
  log lines unredacted.
- R5. WHEN "Exportar diagnóstico (.zip)" is pressed, THE app SHALL write
  `vfs-informe-<AAAA-MM-DD-HHMM>.zip` (holding `informe.txt`, the same text as the preview) to
  the user's Downloads folder (Desktop, then home, when it does not exist), reveal it
  selected in Explorer/Finder, and say where it is and to send it to whoever installed VFS
  for them or attach it on GitHub.
- R6. WHEN "Enviar por GitHub" is pressed, THE app SHALL do what R5 does and then open in the
  default browser a new issue on the project from the `comentarios` issue form, prefilled
  with the user's text and the report summary (everything but the log lines and the root
  paths, which go only in the `.zip` even with the names box ticked), the URL kept at most
  8,000 characters by shortening the user's text if needed; the form asks the user to drag
  the `.zip` into the issue.
- R7. THE app SHALL keep up to 11 log files of 2 MB (22 MB at most), enough for several
  weeks of history on the reference machine, and SHALL write every panic, with its source
  location, to the log as an error before the default panic behavior runs.
- R8. THE app SHALL send nothing by itself: no network request is made by the feature;
  opening the browser is the user's own action.

## Acceptance scenarios

- AC-1 (R2) Given an index with one root and a log, When the report is built, Then it holds
  the version, OS, the root's drive kind, disk kind, status and counts, the index stats, the
  settings, the log-level counts and the log lines, oldest file first.
- AC-2 (R3) Given log warnings in every format the code writes today (a root path, a folder
  under a root, a Windows path with spaces, a UNC path, a macOS path, a Firefox profile
  folder, a path inside an error message) and a panic message naming a file, When the report
  is built with the box unticked, Then no root path, file name or folder name from them
  appears, `<ruta>` takes their place, and diagnostic lines are byte-for-byte unchanged.
- AC-3 (R4) Given the same log, When the box is ticked, Then the root paths and the original
  lines appear.
- AC-4 (R5) Given a downloads folder, When the report is exported, Then a `.zip` named by the
  date and time exists there whose `informe.txt` equals the previewed text; and the folder
  falls back to Desktop, then home, when missing.
- AC-5 (R6) Given a user text of 20,000 characters, When the GitHub URL is built, Then it
  points to `issues/new?template=comentarios.yml`, carries the text (shortened) and the
  summary but no log line and no root path (even with the names box ticked), is at most
  8,000 characters, and every field value is percent-encoded; the issue form file declares
  the fields the URL fills.
- AC-6 (R7) Given the logger and the panic hook installed, When a thread panics, Then the log
  file holds an error naming the panic and its location; and the logger rotates at 2 MB
  keeping 10 old files.
- AC-7 (R1) Given the app, When "Enviar comentarios" is pressed, Then the dialog opens with
  the box unticked, "Ver el informe" shows the report text, and ticking the box refreshes it.

## Non-goals

- Sending anything automatically: no telemetry, no crash upload, no project email address
  (decision 1); a `mailto:` can be added later in one line.
- Screenshots, attaching index or user files, or reading file contents.
- Detecting SSD/HDD on Linux, or for network shares (reported as unknown).
- Translating the report: it is a technical document read by maintainers; its labels are
  English and its log lines as written. The dialog's own strings go through the UI catalog.

## Pre-mortem

- Slow PC / HDD: the report is built only on demand, reads at most 22 MB of log once, and
  compresses it; the dialog's preview loads off the main thread (async command).
- Long paths, Unicode, NFC/NFD: redaction works on the text of each line; a root path is
  matched as written in the log and in the index (both come from the same `Path`), and the
  generic path rule catches the rest.
- Network/removable drives: disk kind reads "unknown"; nothing blocks on a missing drive (the
  OS query has no timeout risk because only the system volume of each root is asked).
- Permissions: nothing needs administrator rights; the zip goes to a user folder; a failed
  write is shown as an error in the dialog.
- User files: never opened, never written; only the log and the index stats are read.
- Antivirus, killed mid-write: a half-written zip is a temporary file renamed at the end.
- A huge log flood: capped by rotation (R7); the URL never holds log lines.
- Privacy leak through a new log format or an unusual name: a path runs to its closing quote
  or the line end, and a Windows path also to the next `": "` (never inside a Windows name;
  every log format puts it before the error text). A macOS name can hold `": "` (a "/" typed
  in Finder), so a POSIX path never ends there and its error text is redacted with it.
  Parentheses and apostrophes never end a path; a known root is always consumed whole; a test
  feeds every log format in the code plus such names.
- Names on a public page: the issue URL never carries the root paths, even with the names box
  ticked; they reach GitHub only if the user attaches the `.zip`, and the dialog says the page
  is public.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Where people without GitHub send the `.zip` | No project email for now: save it, reveal it, tell them to give it to whoever installed VFS or attach it on GitHub | owner | 2026-11-21 |
| 2 | Report format | One `informe.txt` inside a `.zip`; English labels | engineering | 2026-11-21 |
| 3 | Log retention | 2 MB × 11 files (was 1 MB × 5, VFS-5); this spec supersedes that number | engineering | 2026-11-21 |
| 4 | Names in the report | Redacted by default; an unticked box includes them | engineering, from the brief | 2026-11-21 |
| 5 | "Exportar diagnóstico" | The dialog's second action; the text box may stay empty | engineering | 2026-11-21 |
