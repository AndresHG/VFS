#!/usr/bin/env python3
"""PreToolUse hook: personal data never leaves through a commit, a PR or an issue.

The repo is public. Before a Bash command that publishes text (`git commit`, or `gh` writing
a PR, issue, release, comment or API call), this hook checks the command text, any
`--body-file`/`-F` file it names, and for `git commit` the staged diff, against the owner's
denylist (kept outside the repo; see scripts/denylist_scan.py). A hit -> "deny", naming the
denylist entry number only.

FAILS OPEN: no denylist, an unparsable payload or a git error lets the command through; CI
(`privacy.yml`) runs the same scan on every PR. Wired in .claude/settings.json (PreToolUse,
matcher "Bash").
"""

import importlib.util
import json
import pathlib
import re
import shlex
import subprocess
import sys

_SCAN = pathlib.Path(__file__).resolve().parents[2] / "scripts" / "denylist_scan.py"

GIT_COMMIT = re.compile(r"\bgit\b(?:\s+-C\s+\S+)?(?:\s+-c\s+\S+)*\s+commit\b")
GH_WRITE = re.compile(r"\bgh\s+(?:pr|issue|release|api|gist)\b")


def _scanner():
    spec = importlib.util.spec_from_file_location("denylist_scan", _SCAN)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def body_files(command, cwd):
    """Paths given to --body-file / -F / --notes-file, resolved against cwd."""
    try:
        words = shlex.split(command)
    except ValueError:
        words = command.split()
    out = []
    for i, w in enumerate(words):
        for flag in ("--body-file", "--notes-file", "-F"):
            if w == flag and i + 1 < len(words):
                out.append(words[i + 1])
            elif w.startswith(flag + "="):
                out.append(w.split("=", 1)[1])
    return [pathlib.Path(cwd or ".", p.lstrip("@")) for p in out if p != "-"]


def problems(command, cwd, scan, patterns):
    msgs = [f"command text line {n}: denylist entry #{i}" for n, i in scan.find(patterns, command)]
    for path in body_files(command, cwd):
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        msgs += [f"{path.name} line {n}: denylist entry #{i}" for n, i in scan.find(patterns, text)]
    if GIT_COMMIT.search(command):
        # `commit -a` also commits unstaged changes to tracked files.
        what = "HEAD" if re.search(r"\scommit\b.*\s(-[a-zA-Z]*a[a-zA-Z]*|--all)\b", command) else "--cached"
        try:
            diff = subprocess.run(
                ["git", "diff", what, "--no-color", "-U0"], cwd=cwd or None,
                capture_output=True, check=True, text=True, encoding="utf-8", errors="replace",
            ).stdout
            msgs += scan.scan_diff(patterns, diff)
        except (OSError, subprocess.CalledProcessError):
            pass
    return msgs


def main():
    try:
        data = json.load(sys.stdin)
        command = data["tool_input"]["command"]
        cwd = data.get("cwd") or ""
    except Exception:
        return
    if not isinstance(command, str) or not (GIT_COMMIT.search(command) or GH_WRITE.search(command)):
        return
    try:
        scan = _scanner()
        patterns = scan.load()
    except Exception:
        return
    if not patterns:
        return
    msgs = problems(command, cwd, scan, patterns)
    if msgs:
        print(json.dumps({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": "Personal data from the owner's denylist would reach "
                "the public repo (rubric R6): " + "; ".join(msgs),
            }
        }))


if __name__ == "__main__":
    main()
