#!/usr/bin/env python3
"""Public-repo hygiene: no string from the owner's private denylist reaches the public repo.

The repo is public. The denylist (names, emails, paths, employers: anything that identifies
the owner or people close to them) lives OUTSIDE the repo, so the list itself never becomes
public: the `VFS_DENYLIST` environment variable (a CI secret) or the file named by
`VFS_DENYLIST_FILE`, defaulting to `~/.config/vfs/denylist`. One entry per line; `#` starts
a comment; an entry matches case-insensitively as a substring, or as a regex when written
`re:<pattern>`.

A hit is reported by location and entry number only, never by the matched text: CI logs
are public too.

Usage (standard library only):
  denylist_scan.py staged        added lines of `git diff --cached`
  denylist_scan.py range BASE    added lines, commit messages and commit emails of BASE...HEAD
  denylist_scan.py text [FILE]   FILE or stdin (a PR body, a commit message, a command)

`range` also fails on a commit whose author or committer email is not a noreply address,
with or without a denylist. With no denylist, the other checks print a notice and pass.
Exit 1 on any hit.
"""

import os
import pathlib
import re
import subprocess
import sys

DEFAULT_FILE = pathlib.Path.home() / ".config" / "vfs" / "denylist"
# Commit emails allowed in a public history: GitHub's per-user noreply, GitHub's own web
# merges, and the agent co-author/bot identities.
NOREPLY = re.compile(
    r"^([^@\s]+@users\.noreply\.github\.com|noreply@github\.com|noreply@anthropic\.com)$",
    re.IGNORECASE,
)


def parse(text):
    """Compiled patterns, in file order, from the denylist text."""
    out = []
    for raw in text.splitlines():
        entry = raw.strip()
        if not entry or entry.startswith("#"):
            continue
        if entry.startswith("re:"):
            out.append(re.compile(entry[3:], re.IGNORECASE))
        else:
            out.append(re.compile(re.escape(entry), re.IGNORECASE))
    return out


def load(env=os.environ):
    """Patterns from VFS_DENYLIST, else VFS_DENYLIST_FILE, else the default file; [] if none."""
    if env.get("VFS_DENYLIST", "").strip():
        return parse(env["VFS_DENYLIST"])
    path = pathlib.Path(env.get("VFS_DENYLIST_FILE") or DEFAULT_FILE)
    try:
        return parse(path.read_text(encoding="utf-8"))
    except OSError:
        return []


def find(patterns, text):
    """(line_number, entry_number) for every line that matches an entry; both 1-based."""
    hits = []
    for n, line in enumerate(text.splitlines(), 1):
        for i, p in enumerate(patterns, 1):
            if p.search(line):
                hits.append((n, i))
    return hits


def _header_path(row):
    """The new-side path of a '+++ ' diff header, or None for /dev/null.

    Git appends a tab to paths holding spaces and C-quotes paths holding unusual bytes.
    """
    s = row[4:].rstrip("\t")
    if s.startswith('"') and s.endswith('"'):
        raw = s[1:-1].encode("latin-1", "backslashreplace").decode("unicode_escape")
        s = raw.encode("latin-1", "replace").decode("utf-8", "replace")
    return s[2:] if s.startswith("b/") else None


def added_lines(diff):
    """{path: [(new_line_number, text)]} for the '+' lines of a unified diff."""
    out, path, line = {}, None, 0
    for row in diff.splitlines():
        if row.startswith("+++ "):
            path = _header_path(row)
        elif row.startswith("@@"):
            m = re.search(r"\+(\d+)", row)
            line = int(m.group(1)) if m else 0
        elif path and row.startswith("+"):
            out.setdefault(path, []).append((line, row[1:]))
            line += 1
        elif path and not row.startswith("-"):
            line += 1
    return out


def scan_changes(patterns, diff, names):
    """Messages for denylisted changed paths (`names`) and added lines (`diff`).

    A path that matches is never printed: the file is named by its position in `names`.
    Paths come from `names`, not the diff, because empty, binary and renamed files have no
    '+++' line.
    """
    msgs, hidden = [], {}
    for k, name in enumerate(names, 1):
        for _, i in find(patterns, name):
            hidden[name] = f"changed file #{k}"
            msgs.append(f"{hidden[name]}: file path matches denylist entry #{i}")
    for path, rows in added_lines(diff).items():
        label = hidden.get(path) or ("a changed file" if find(patterns, path) else path)
        for n, text in rows:
            for _, i in find(patterns, text):
                msgs.append(f"{label}:{n}: matches denylist entry #{i}")
    return msgs


def git(*args, cwd=None):
    return subprocess.run(
        ["git", "-c", "core.quotePath=false", *args], cwd=cwd, capture_output=True, check=True,
        text=True, encoding="utf-8", errors="replace",
    ).stdout


def scan_git(patterns, *spec, cwd=None):
    """scan_changes over `git diff <spec>`: e.g. ("--cached",), ("HEAD",), ("BASE...HEAD",)."""
    diff = git("diff", "--no-color", "-U0", *spec, cwd=cwd)
    names = [n for n in git("diff", "--name-only", "-z", *spec, cwd=cwd).split("\0") if n]
    return scan_changes(patterns, diff, names)


def scan_range(patterns, base):
    msgs = []
    log = git("log", "--format=%H%x00%ae%x00%ce%x00%B%x01", f"{base}..HEAD")
    for record in filter(None, (r.strip("\n") for r in log.split("\x01"))):
        sha, author, committer, body = record.split("\x00", 3)
        short = sha[:9]
        for who, email in (("author", author), ("committer", committer)):
            if not NOREPLY.search(email.strip()):
                msgs.append(f"commit {short}: {who} email is not a noreply address")
        for n, i in find(patterns, body):
            msgs.append(f"commit {short} message line {n}: matches denylist entry #{i}")
    if patterns:
        msgs += scan_git(patterns, f"{base}...HEAD")
    return msgs


def main(argv):
    if not argv or argv[0] not in ("staged", "range", "text"):
        print(__doc__)
        return 2
    patterns = load()
    mode = argv[0]
    if not patterns:
        print("denylist_scan: no denylist configured; only commit emails are checked.")
    if mode == "staged":
        msgs = scan_git(patterns, "--cached") if patterns else []
    elif mode == "range":
        if len(argv) < 2:
            print("usage: denylist_scan.py range BASE")
            return 2
        msgs = scan_range(patterns, argv[1])
    else:
        if len(argv) > 1:
            text = pathlib.Path(argv[1]).read_text(encoding="utf-8", errors="replace")
        else:
            text = sys.stdin.read()
        msgs = [f"text line {n}: matches denylist entry #{i}" for n, i in find(patterns, text)]
    for m in msgs:
        print(m)
    if msgs:
        print("Personal data must not reach the public repo; remove it (rubric R6).")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
