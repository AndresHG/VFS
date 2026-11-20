#!/usr/bin/env python3
"""PostToolUse hook — CLAUDE.md budget sensor (ported from Calligence).

After every Edit/Write/MultiEdit of a CLAUDE.md inside the project, check it
against its budget and the growth rule — the policy in
scripts/claude_md_budget.py — and inject what it breaks back into the loop,
so the file is trimmed before the push. Silent on success: a sensor reports
failures only.

The file's own git checkout is its root, so a linked worktree under
.claude/worktrees/ is measured as itself, against its own BUDGETS. Fails
open: garbage input, a file outside the project, or a policy that can't load
says nothing; without git or origin/main only the cap is checked. Wired in
.claude/settings.json (PostToolUse, matcher "Edit|Write|MultiEdit").
"""

import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path

POLICY = Path("scripts") / "claude_md_budget.py"


def load_policy(root):
    """The edited checkout's own policy (its branch's BUDGETS), else this hook's."""
    source = root / POLICY
    if not source.is_file():
        source = Path(__file__).resolve().parents[2] / POLICY
    spec = importlib.util.spec_from_file_location("claude_md_budget", source)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def checkout_root(path):
    try:
        proc = subprocess.run(
            ["git", "-C", str(path.parent), "rev-parse", "--show-toplevel"],
            capture_output=True, text=True, timeout=10,
        )
    except Exception:
        return None
    return Path(proc.stdout.strip()).resolve() if proc.returncode == 0 else None


def main():
    try:
        data = json.load(sys.stdin)
    except Exception:
        return
    file_path = (data.get("tool_input") or {}).get("file_path") or ""
    project_dir = os.environ.get("CLAUDE_PROJECT_DIR") or ""
    if os.path.basename(file_path) != "CLAUDE.md" or not os.path.isfile(file_path) or not project_dir:
        return
    project, path = Path(project_dir).resolve(), Path(file_path).resolve()
    if project not in path.parents:
        return
    root = checkout_root(path) or project
    if root != project and project not in root.parents:
        return
    rel = path.relative_to(root).as_posix()
    if rel.startswith(".claude/worktrees/"):
        return  # a worktree git can't resolve: its budgets are unknowable here

    try:
        policy = load_policy(root)
        problems = policy.cap_problems(rel, path.read_text(encoding="utf-8"))
        base = policy.merge_base(root)
        if base:
            before = policy.size_at(base, rel, root)
            growth = policy.growth_problem(rel, len(path.read_bytes()), before)
            if growth:
                problems.append(growth)
    except Exception:
        return
    if not problems:
        return

    msg = (
        "CLAUDE.md budget check on the file you just edited (policy and full check: "
        "`python3 scripts/claude_md_budget.py`):\n- " + "\n- ".join(problems)
    )
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": msg,
        }
    }))


if __name__ == "__main__":
    main()
