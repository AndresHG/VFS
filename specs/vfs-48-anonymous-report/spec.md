# VFS-48 — Diagnostic report: always anonymous

- **Status**: in progress
- **Size**: M (one module, `feedback.rs`, its dialog and copy; no schema change)
- **Links**: supersedes VFS-20 R3's names choice (the "names" box) and VFS-30 R1–R3's
  "WHILE the names box is unticked" condition: they now always hold; VFS-20 R6 (prefilled
  public issue) stays

## Problem

"Enviar comentarios" leads to a public GitHub issue with the report attached. The dialog let
the user tick a box to include folder and file names, and the user's own text went into the
report and the issue as written. Reports attached to public issues by testers showed their
user account name, a drive folder named after a person, an institution's synced folder and a
corporate user id: the box was ticked, or the text named a folder. The owner's decision is
to keep the public issue but make the report anonymous whatever the user does.

## Requirements (EARS)

- R1. THE report SHALL never include a root path, a file or folder name, or the user's
  account name: the dialog has no box to include them, and every log line is redacted as
  VFS-30 R1–R3 describe.
- R2. THE report and the prefilled issue SHALL carry the user's "what happened" text with
  every path replaced by `<ruta>` (the VFS-20 R3 rule, roots and home folder included).
- R3. THE report SHALL replace the user's account name (the home folder's last part, when
  it is at least 3 characters) by `<usuario>` wherever it stands as a whole word, in the
  user's text and in the log lines, matching case-insensitively.
- R4. THE dialog SHALL say that the report is anonymous and that the issue is public, so
  the user leaves personal details out of the text.

Diagnostic records (counts and durations), the OS, the app version and the per-root counts
stay: they hold no names (VFS-5 R8, VFS-37 R1). Free text the user types that names a person
or an institution without a path cannot be detected; R4 covers it with a warning.

## Acceptance criteria

- AC-1 (R1): with roots holding a person's name and an institution's synced folder, the
  saved report holds neither, has no `path:` line and no `names_included` line.
- AC-2 (R2): a "what happened" text with a home path, a root path and a Windows path holds
  `<ruta>` instead of each, in the report and in the issue URL.
- AC-3 (R3): the account name alone (any case) in the text and in a warning becomes
  `<usuario>`; a longer word containing it is kept; a 2-character account name is ignored.
- AC-4 (R1, R4): the dialog has no names box, asks for the report and saves it without a
  names argument, and shows the anonymous/public notice in Spanish and English.

## Cost (R11)

One extra pass over the user's text and over each redacted log message (a whole-word scan
for one short string): negligible next to reading the log. No new dependency, no disk I/O.
