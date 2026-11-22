# VFS-30 — Diagnostic report: no free text from panics, bounded preview

- **Status**: approved (follow-up to VFS-20, from an independent pre-release review)
- **Size**: M (one module, `feedback.rs`, and its dialog; no schema change)
- **Links**: supersedes VFS-20 R1 (the preview), R3 (redaction) and R5 (what the `.zip`
  holds); VFS-20 AC-2 and AC-4 still hold as written, AC-7 is extended by AC-5 here

## Problem

An independent review of VFS-20 before release found that, with the names box unticked, the
saved report could still carry names:

- A panic's payload is free text. Rust's own messages quote values in backticks (a slicing
  panic reads ``… of `D:\Archivo\Boda año.jpg` ``), and a payload can hold a bare name or a
  search term with no path around it. VFS-20 only redacted what reads as a path, and only
  after a space, tab, quote, `(`, `[`, `=`, `:` or `,`.
- A macOS name can hold a line break, so the tail of a name lands on a line with no log
  prefix, which VFS-20 redacted like any message.

The review also found that "Ver el informe" put the whole report, up to 22 MB of log, into
a text box, and rebuilt it whenever the user's text changed: slow on the reference PC.

## Requirements (EARS)

- R1. WHILE the names box is unticked, THE report SHALL replace a panic record's message
  (target `vfs::panic`) with `<omitido>` and keep only `panic at <file>:<line>`; a panic line
  whose location cannot be read SHALL be `<omitido>` whole.
- R2. WHILE the names box is unticked, THE report SHALL treat as the start of a path, in
  every non-diagnostic line, a drive, UNC, `/` or `~/` prefix or a known root after a space,
  tab, quote, backtick, `(`, `[`, `{`, `<`, `|`, `=`, `:` or `,`, and replace it with
  `<ruta>` as VFS-20 R3 says.
- R3. WHILE the names box is unticked, THE report SHALL keep a continuation line (one with no
  `[date][time][target][LEVEL]` prefix) as written only when the record it continues is a
  diagnostic one (`vfs::diag`), and SHALL replace any other continuation line with
  `<omitido>` whole.
- R4. "Ver el informe" SHALL show the report with only its newest 200 KB of log lines, whole
  lines only, with a line in the report saying older lines are left out, while the log
  counts still cover every line; a line in the dialog (catalog, es and en) SHALL say the
  `.zip` holds the whole report. The `.zip` SHALL hold the whole report.
- R5. THE dialog SHALL build the preview only when "Ver el informe" is opened or the names
  box changes, never when the user's text changes.

## Acceptance scenarios

Tests cite these ids (`VFS-30 AC-<m>`) in their doc comments.

- AC-1 (R1, R2, R3) Given the VFS-20 AC-2 log plus a slicing panic quoting a path in
  backticks, a panic naming a bare search term, warnings with a path after a backtick and
  after `<`, and a warning whose macOS name holds a line break, When the report is built
  with the box unticked, Then none of those names appears, each panic keeps its location
  followed by `<omitido>`, and the continuation line is `<omitido>`.
- AC-2 (R2) Given messages with a path after a backtick, `<`, `{` and `|`, When redacted,
  Then each path is `<ruta>`.
- AC-3 (R1, R3) Given a panic line with a Windows location, a panic line without a readable
  location, a continuation after a warning and one after a diagnostic record, When redacted,
  Then the first keeps its location, the second is `<omitido>`, the third is `<omitido>`
  and the fourth is unchanged.
- AC-4 (R1, R2) Given the logger installed by the app, When it writes a warning, a diagnostic
  record and a panic, Then the report's parser reads each line's target, level and message,
  keeps the diagnostic line and leaves only the panic's location.
- AC-5 (R4, R5) Given several MB of log, When the preview is built, Then it holds the summary
  and at most 200 KB of the newest log lines (plus headers), says older lines are left out,
  and its counts cover every line, while the full report holds the oldest line; the dialog
  has the catalog line in es and en; and typing in the text box asks for no report.

## Non-goals

- Redacting a bare name in a warning that has no path around it: no `warn!` or `error!`
  format in the code writes one; a new format with free text needs its own rule.
- Changing what the report holds with the names box ticked (VFS-20 R4).

## Pre-mortem

- Slow PC / HDD: the preview still reads the whole log once to count lines (cheap, prefix
  parsing only) and redacts only the 200 KB it shows; the text box never holds more.
- Over-redaction: a panic loses its message even when it named nothing; the location is
  enough to find the code, and the user can tick the names box to send the message.
- A multi-line diagnostic record: kept whole, since VFS-5 R8 keeps names out of them.
- Bound slack: a short continuation line becomes the longer `<omitido>`, and each file adds
  a header line; the preview is "about" 200 KB, a test allows 1 KB over.

## Decisions

| # | Question | Choice | Who | Date |
|---|---|---|---|---|
| 1 | Refresh the preview on typing? | No: the dialog says to close and reopen the section to see the text updated | coordinator review | 2026-11-22 |
