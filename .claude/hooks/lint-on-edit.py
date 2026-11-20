#!/usr/bin/env python3
"""PostToolUse hook — format sensor for edited Rust files.

After every Edit/Write/MultiEdit of a *.rs file inside the project, run the
formatter `cargo fmt` uses (rustfmt --check) on that one file and inject what
it would change back into the loop as context. Silent on success: a sensor
reports failures only (ported from Calligence's ruff sensor).

The edition comes from the nearest Cargo.toml that states one, else 2021.
rustfmt also checks out-of-line modules the file declares (`mod x;`).
Frontend files are left alone for now.

Fails open: if cargo/rustfmt is not installed, times out, or the input is
garbage, say nothing. Wired in .claude/settings.json (PostToolUse, matcher
"Edit|Write|MultiEdit").
"""

import json
import os
import re
import shutil
import subprocess
import sys

DEFAULT_EDITION = "2021"
EDITION = re.compile(r'^\s*edition\s*=\s*"(\d{4})"', re.MULTILINE)
MAX_OUTPUT = 2000


def edition_for(file_path, root):
    """The edition of the nearest Cargo.toml above file_path that states one."""
    d = os.path.dirname(os.path.abspath(file_path))
    while True:
        manifest = os.path.join(d, "Cargo.toml")
        if os.path.isfile(manifest):
            with open(manifest, encoding="utf-8") as f:
                match = EDITION.search(f.read())
            if match:
                return match.group(1)
        parent = os.path.dirname(d)
        if d == root or parent == d:
            return DEFAULT_EDITION
        d = parent


def main():
    try:
        data = json.load(sys.stdin)
        file_path = (data.get("tool_input") or {}).get("file_path") or ""
    except Exception:
        return
    if not isinstance(file_path, str) or not file_path.endswith(".rs") or not os.path.isfile(file_path):
        return

    # Scratch files outside the project are not ours to check.
    root = os.path.abspath(os.environ.get("CLAUDE_PROJECT_DIR") or "")
    if root and not os.path.abspath(file_path).startswith(root + os.sep):
        return

    # Rust may not be installed yet: degrade silently.
    rustfmt = shutil.which("rustfmt")
    if not shutil.which("cargo") or not rustfmt:
        return
    try:
        edition = edition_for(file_path, root)
        proc = subprocess.run(
            [rustfmt, "--check", "--edition", edition, file_path],
            cwd=os.path.dirname(os.path.abspath(file_path)),
            capture_output=True, text=True, timeout=30,
        )
    except Exception:
        return
    findings = (proc.stdout + proc.stderr).strip()
    if proc.returncode == 0 or not findings:
        return

    if len(findings) > MAX_OUTPUT:
        findings = findings[:MAX_OUTPUT] + "\n… (truncated)"
    msg = (
        "rustfmt --check (what `cargo fmt --check` runs in CI) flagged the file you "
        f"just edited:\n{findings}\nRun `cargo fmt` (or fix the syntax error it "
        "reports) before committing."
    )
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": msg,
        }
    }))


if __name__ == "__main__":
    main()
