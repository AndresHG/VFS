#!/usr/bin/env python3
"""The CLAUDE.md budget policy: per-file caps and the growth rule (ported from an earlier project).

Standard library only, so a bare python3 can run it. `python3 scripts/claude_md_budget.py`
checks every CLAUDE.md in the checkout and exits 1 on a breach (for CI and local runs);
.claude/hooks/claude-md-budget-on-edit.py reports the same policy the moment a session
edits a CLAUDE.md.
"""

import pathlib
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_MAX_BYTES = 4_000
DEFAULT_MAX_LINE = 400
HEADROOM_BYTES = 1_000
# Growth is measured from where a change left main: a PR's own diff, and nothing on main itself.
GROWTH_BASE = "origin/main"

# path -> (max_bytes, max_line_chars). Lower these as files are pruned;
# never raise one without a reason in the PR. Unlisted files take the default.
BUDGETS = {
    "CLAUDE.md": (10_000, 400),
}


def claude_md_files(repo=REPO):
    """Every CLAUDE.md in this checkout, skipping worktrees and dependency trees.

    Exclusions match the path RELATIVE to the repo: a linked worktree lives under
    `.claude/worktrees/`, so filtering on the absolute path would empty this list.
    """
    skip = (".claude/worktrees/", "node_modules/", "target/")
    found = []
    for path in repo.rglob("CLAUDE.md"):
        rel = path.relative_to(repo).as_posix()
        if any(rel.startswith(s) or f"/{s}" in rel for s in skip):
            continue
        found.append(path)
    return sorted(found)


def budget_for(rel):
    return BUDGETS.get(rel, (DEFAULT_MAX_BYTES, DEFAULT_MAX_LINE))


def cap_problems(rel, text):
    """What `text` breaks of the cap and the line limit for `rel`; empty when it fits."""
    max_bytes, max_line = budget_for(rel)
    size = len(text.encode("utf-8"))
    longest = max((len(line) for line in text.splitlines()), default=0)
    problems = []
    if size > max_bytes:
        problems.append(
            f"{rel} is {size} B, over its {max_bytes} B budget. Prune it — a rule that "
            "changed is edited in place, never appended; narrative goes to the PR body "
            "or a spec. Raise the budget only with a reason in the PR."
        )
    if longest > max_line:
        problems.append(
            f"{rel} has a {longest}-character line, over its {max_line} limit. One "
            "present-tense rule per bullet, with the spec or PR that earned it."
        )
    return problems


def growth_problem(rel, size, before):
    """Why growing `rel` from `before` to `size` bytes is refused, or None.

    A change may grow a file only while it stays HEADROOM_BYTES under its cap, and an
    existing file by at most HEADROOM_BYTES; inside that last room it may edit the file
    but not grow it. Two PRs that each pass then merge within the cap, however far under
    it the file starts: a check that two passing PRs can break together must gate the
    change, not the result. A new file is exempt from the step limit.
    """
    max_bytes = budget_for(rel)[0]
    limit = max_bytes - HEADROOM_BYTES
    if size <= before:
        return None
    fix = "edit a rule in place or drop one that no longer holds; narrative goes to the PR body or a spec."
    if size > limit:
        return (
            f"{rel} grew from {before} B to {size} B, into the last {HEADROOM_BYTES} B of its "
            f"{max_bytes} B budget, which stays free so parallel PRs cannot merge over it. "
            f"Remove {size - max(before, limit)} B: {fix}"
        )
    if before and size - before > HEADROOM_BYTES:
        return (
            f"{rel} grew by {size - before} B in one change; two such changes could merge over "
            f"its {max_bytes} B budget. Remove {size - before - HEADROOM_BYTES} B: {fix}"
        )
    return None


def _git(repo, *args):
    try:
        proc = subprocess.run(
            ["git", "-C", str(repo), *args], capture_output=True, text=True, timeout=10
        )
    except (OSError, subprocess.SubprocessError):
        return None
    return proc.stdout.strip() if proc.returncode == 0 else None


def merge_base(repo=REPO):
    """The commit where this checkout left main, or None without git or GROWTH_BASE."""
    return _git(repo, "merge-base", GROWTH_BASE, "HEAD") or None


def size_at(commit, rel, repo=REPO):
    """`rel`'s size in bytes at `commit`; 0 for a file that did not exist there."""
    out = _git(repo, "cat-file", "-s", f"{commit}:{rel}")
    return int(out) if out else 0


def check(repo=REPO):
    """Every breach in the checkout: caps always, growth when GROWTH_BASE is known."""
    base = merge_base(repo)
    problems = []
    for path in claude_md_files(repo):
        rel = path.relative_to(repo).as_posix()
        problems += cap_problems(rel, path.read_text(encoding="utf-8"))
        if base:
            growth = growth_problem(rel, len(path.read_bytes()), size_at(base, rel, repo))
            if growth:
                problems.append(growth)
    return problems


if __name__ == "__main__":
    found = check()
    for problem in found:
        print(f"- {problem}")
    sys.exit(1 if found else 0)
