#!/usr/bin/env python3
"""Stop hook — a session that changed code ends with a PR.

Root CLAUDE.md gives standing authorization: code changed on a feature branch
→ commit, run the pr-critic, push, open a PR into main. Claude Code's default
("commit or push only when the user asks") leaves sessions stopping at "want
me to commit?", and the PR never happens. This hook turns the rule into a
check at the moment the session tries to end its turn (ported from an earlier project).

Blocks the stop ONCE (`decision: block`) when the branch differs from
origin/main and any of these hold:
  - the working tree has uncommitted changes,
  - commits are not pushed (no origin/<branch>, or ahead of it),
  - no PR exists for the branch,
  - its PR is open and the body has no `## Lessons` section: what the
    session learned and where each lesson went, or `None`.
The second stop of the same turn arrives with `stop_hook_active` and is let
through, so a session that is legitimately waiting on the user (a question,
a blocker, the user said not to push) says so and ends.

Silent when the repository has no remote yet, on main, on a detached HEAD, on
a branch identical to origin/main, and FAILS OPEN on garbage input or when
git/gh can't answer. Wired in .claude/settings.json (Stop).
"""

import json
import re
import subprocess
import sys

PROTECTED = {"main"}
BASE = "origin/main"
NO_LESSONS = "its PR body has no filled-in `## Lessons` section"
LESSONS_HEADING = re.compile(r"^#{2,3}[ \t]*Lessons\b.*$", re.IGNORECASE | re.MULTILINE)
NEXT_HEADING = re.compile(r"^#{1,3}[ \t]", re.MULTILINE)
HTML_COMMENT = re.compile(r"<!--.*?-->", re.DOTALL)


def lessons_lines(body):
    """The text lines under every Lessons heading, without HTML comments or the 🤖 line.

    The attribution line closing every PR body is not a lesson; neither is the
    template's comment.
    """
    found = []
    for heading in LESSONS_HEADING.finditer(body):
        rest = body[heading.end():]
        end = NEXT_HEADING.search(rest)
        section = rest[:end.start()] if end else rest
        found += [line for line in HTML_COMMENT.sub("", section).splitlines()
                  if line.strip() and not line.lstrip().startswith("🤖")]
    return found


def has_lessons(body):
    """True when a Lessons heading has text under it: `None` counts, the template's comment doesn't."""
    return bool(lessons_lines(body))


def _run(args, cwd, timeout=10):
    """stdout of a command, or None when it fails or can't run."""
    try:
        proc = subprocess.run(args, cwd=cwd or None, capture_output=True, text=True, timeout=timeout)
    except Exception:
        return None
    return proc.stdout.strip() if proc.returncode == 0 else None


def _pr(cwd):
    """(state, body): state 'OPEN' / 'MERGED' / 'CLOSED', '' when the branch has no PR,
    None when unknown; body None when unknown."""
    try:
        proc = subprocess.run(
            ["gh", "pr", "view", "--json", "state,body"],
            cwd=cwd or None, capture_output=True, text=True, timeout=15,
        )
    except Exception:
        return None, None
    if proc.returncode != 0:
        return ("" if "no pull requests found" in proc.stderr.lower() else None), None
    try:
        pr = json.loads(proc.stdout)
    except ValueError:
        return None, None
    if not isinstance(pr, dict):
        return None, None
    body = pr.get("body")
    return pr.get("state"), body if isinstance(body, str) else None


def problems(cwd):
    """What still separates this branch from a PR; [] when nothing does."""
    if not _run(["git", "remote"], cwd):
        return []  # no remote yet (or git can't answer): there is nowhere to open a PR
    branch = _run(["git", "symbolic-ref", "--short", "HEAD"], cwd)
    if not branch or branch in PROTECTED:
        return []
    ahead_of_base = _run(["git", "rev-list", "--count", f"{BASE}..HEAD"], cwd)
    status = _run(["git", "status", "--porcelain"], cwd)
    if ahead_of_base is None or status is None:
        return []
    dirty = bool(status)
    if not dirty and ahead_of_base == "0":
        return []  # nothing was changed on this branch

    found = []
    if dirty:
        found.append("the working tree has uncommitted changes")
    if ahead_of_base == "0":
        return found
    state, body = _pr(cwd)
    # Compare with the branch's own remote ref, never @{upstream}: worktree
    # branches start tracking origin/main, and a push without -u keeps it so.
    remote = _run(["git", "rev-parse", "--verify", "-q", f"refs/remotes/origin/{branch}"], cwd)
    if remote is None:
        if state in ("", None):  # MERGED/CLOSED + pruned remote: the work landed
            found.append("the branch was never pushed")
    else:
        unpushed = _run(["git", "rev-list", "--count", f"origin/{branch}..HEAD"], cwd)
        if unpushed and unpushed != "0":
            found.append(f"{unpushed} commit(s) are not pushed")
        if state == "":
            found.append("no PR exists for the branch")
    if state == "OPEN" and body is not None and not has_lessons(body):
        found.append(NO_LESSONS)
    if state == "MERGED" and found:
        found.append("its PR is already merged, so new pushes need a new PR")
    return found


def main():
    try:
        data = json.load(sys.stdin)
    except Exception:
        return
    if not isinstance(data, dict) or data.get("stop_hook_active"):
        return
    cwd = data.get("cwd") if isinstance(data.get("cwd"), str) else ""
    found = problems(cwd)
    if not found:
        return
    branch = _run(["git", "symbolic-ref", "--short", "HEAD"], cwd)
    if found == [NO_LESSONS]:
        lead = (f"The PR for branch '{branch}' is open, but {NO_LESSONS}. Add it with "
                "`gh pr edit --body-file`.")
    else:
        lead = (f"Branch '{branch}' has changes, but {'; '.join(found)}. Root CLAUDE.md: a "
                "session that changes code ends with a PR into main. Commit, run the "
                "pr-critic agent, push and open the PR.")
    print(json.dumps({
        "decision": "block",
        "reason": (
            f"{lead} The PR body carries a `## Lessons` "
            "section: one line per user correction, wrong assumption that cost "
            "time, or failure a control could have caught, each saying where it went "
            "(hook, test, rubric line, CLAUDE.md or spec: a path in this PR; auto-memory: "
            "its name; or a new VFS-n spec), routed by root CLAUDE.md's ratchet section; write "
            "`None` when there were none, never invent one. If the user said not to, or "
            "the work is unfinished or waiting on the user, say that in one line and stop."
        ),
    }))


if __name__ == "__main__":
    main()
