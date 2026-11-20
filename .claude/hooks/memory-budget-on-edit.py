#!/usr/bin/env python3
"""PostToolUse hook — auto-memory index budget sensor (ported from Calligence).

Claude Code loads a project's auto-memory index (`MEMORY.md`) into every
session, relevant or not, so each line costs context forever. The CLAUDE.md
files have a budget (scripts/claude_md_budget.py); without one, Calligence's
index reached ~15 KB.

After any Edit/Write to a file in a `~/.claude/projects/<project>/memory/`
directory, measure that directory's `MEMORY.md` and inject what it breaks:
  - REVIEW_AT (90%) of MAX_BYTES or more in total,
  - an index line longer than MAX_LINE characters.
The fix it asks for: consolidate the index (promote lessons that keep coming
back into a control, merge duplicates), or shorter index lines.

Silent on success and for files outside a memory directory; fails open.
Wired in .claude/settings.json (PostToolUse, matcher "Edit|Write|MultiEdit").
"""

import json
import sys
from pathlib import Path

MAX_BYTES = 12_000
MAX_LINE = 150
# From this share of MAX_BYTES on, the message asks for a consolidation,
# so the index is consolidated before it is over budget, not after.
REVIEW_AT = 0.9


def memory_index(file_path):
    """The MEMORY.md of the auto-memory directory holding file_path, or None."""
    path = Path(file_path).expanduser()
    parts = path.parts
    if len(parts) < 5 or parts[-2] != "memory" or parts[-4] != "projects" or parts[-5] != ".claude":
        return None
    index = path.parent / "MEMORY.md"
    return index if index.is_file() else None


def problems(index):
    text = index.read_text(encoding="utf-8")
    found = []
    size = len(text.encode("utf-8"))
    if size > MAX_BYTES:
        found.append(f"{index.name} is {size} bytes, over its {MAX_BYTES}-byte budget")
    elif size >= REVIEW_AT * MAX_BYTES:
        found.append(f"{index.name} is {size} bytes, {size * 100 // MAX_BYTES}% of its {MAX_BYTES}-byte budget")
    long_lines = [n for n, line in enumerate(text.splitlines(), 1) if len(line) > MAX_LINE]
    if long_lines:
        shown = ", ".join(map(str, long_lines[:8])) + ("…" if len(long_lines) > 8 else "")
        found.append(f"{len(long_lines)} index line(s) exceed {MAX_LINE} chars (lines {shown})")
    return found


def main():
    try:
        data = json.load(sys.stdin)
        file_path = (data.get("tool_input") or {}).get("file_path") or ""
        index = memory_index(file_path) if isinstance(file_path, str) and file_path else None
        found = problems(index) if index else []
    except Exception:
        return
    if not found:
        return
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": (
                f"Memory budget: {'; '.join(found)}. The index loads into every "
                "session. Consolidate it, on the user's OK: a lesson cited by two or more "
                "merged PRs' `## Lessons` becomes a control (hook, test, rubric or CLAUDE.md "
                "line, per root CLAUDE.md's ratchet); merge duplicates. For a quick trim, "
                "shorten each index line to what decides relevance. Never shorten by "
                "dropping a feedback rule or deleting a lesson."
            ),
        }
    }))


if __name__ == "__main__":
    main()
