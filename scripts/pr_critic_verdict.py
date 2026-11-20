#!/usr/bin/env python3
"""The pr-critic's verdict for a commit, kept per worktree, and its history (ported from Calligence).

The pr-critic agent reviews a branch's diff in a fresh context and records
its verdict for the exact HEAD it reviewed:

    python3 scripts/pr_critic_verdict.py record pass  "<one-line summary>"
    python3 scripts/pr_critic_verdict.py record block "<one-line summary>" --finding R3=crates/x/src/lib.rs ...

`waived` is recorded by the writer only when the user explicitly skipped the
review. guard-git-danger.py reads it through `gate_reason()` and denies
`gh pr create` until the verdict for the current HEAD is `pass` or `waived`.
A branch whose diff against origin/main is docs only needs no verdict, unless
it touches a file that steers the agents: any CLAUDE.md / AGENTS.md /
SKILL.md, or anything under .claude/ (the rubric, the critic's own definition).

The verdict file lives in the worktree's git dir (`git rev-parse --git-path`),
so it is never committed and one worktree's verdict never unlocks another's PR.
Every `record` also appends a line to `pr-critic-log.jsonl` in the git common
dir, shared by every worktree of the checkout. `report` reads it and
proposes, never applies, exempting a change category or pruning a rubric line.
"""

import argparse
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timezone

FILENAME = "pr-critic.json"
LOG_FILENAME = "pr-critic-log.jsonl"
VERDICTS = ("pass", "block", "waived")
UNLOCKING = ("pass", "waived")
REVIEWED = ("pass", "block")
BASE = "origin/main"
HERE = os.path.dirname(os.path.abspath(__file__))
RUBRIC = os.path.join(HERE, "..", ".claude", "review-rubric.md")
RULE_ID = re.compile(r"^- \*\*(R\d+)\.", re.MULTILINE)
FINDING = re.compile(r"^(R\d+)=(\S+)$")
# A category reviewed on this many branches with no block is proposed for exemption,
# and a rubric id no block cited by then is proposed for pruning.
PROPOSE_AFTER = 20
# Markdown and specs are docs; markdown that instructs agents rather than describing
# the system is not exempt, because a change to it changes how every session works.
DOCS_PATTERN = re.compile(r"(^(docs|specs)/)|(\.md$)")
STEERS_AGENTS = re.compile(r"(^|/)(CLAUDE|AGENTS|SKILL)\.md$|^\.claude/")
AGENT_STEERING, DOCS, OTHER = "agent-steering", "docs", "other"
# Checked in order after agent-steering markdown and docs; the first match wins.
CATEGORIES = (
    ("tests", re.compile(r"(^|/)tests/|_tests?\.rs$")),
    ("hooks-scripts", re.compile(r"^(\.claude|scripts)/")),
    ("build-config", re.compile(
        r"(^|/)(Cargo\.(toml|lock)|rust-toolchain(\.toml)?|package(-lock)?\.json|tauri\.conf\.json"
        r"|build\.rs)$|^\.github/")),
    ("rust", re.compile(r"\.rs$")),
    ("frontend", re.compile(r"\.(html|css|js|mjs|ts|tsx)$")),
)
NEVER_EXEMPT = (DOCS, OTHER)  # docs are exempt already; `other` is too vague to exempt


def _git(cwd, *args):
    try:
        proc = subprocess.run(["git", *args], cwd=cwd or None, capture_output=True, text=True, timeout=10)
    except Exception:
        return None
    return proc.stdout.strip() if proc.returncode == 0 else None


def verdict_path(cwd):
    path = _git(cwd, "rev-parse", "--path-format=absolute", "--git-path", FILENAME)
    return path or None


def log_path(cwd):
    common = _git(cwd, "rev-parse", "--path-format=absolute", "--git-common-dir")
    return os.path.join(common, LOG_FILENAME) if common else None


def rubric_ids():
    """The blocker ids in .claude/review-rubric.md, in order."""
    with open(RUBRIC, encoding="utf-8") as f:
        return RULE_ID.findall(f.read())


def categorize(path, docs=None):
    """The change category a path counts under in the verdict log."""
    docs = docs or DOCS_PATTERN
    if docs.search(path):
        return AGENT_STEERING if STEERS_AGENTS.search(path) else DOCS
    return next((name for name, pattern in CATEGORIES if pattern.search(path)), OTHER)


def _changed(cwd):
    changed = _git(cwd, "diff", "--name-only", "--no-renames", f"{BASE}...HEAD")
    return changed.splitlines() if changed else []


def parse_findings(verdict, findings):
    """`R<n>=<path>` citations as (rule, path); a block needs one, nothing else takes any."""
    if verdict != "block":
        if findings:
            raise SystemExit(f"only a block cites findings, not {verdict}")
        return []
    if not findings:
        raise SystemExit("a block cites at least one finding: --finding R<n>=<path>")
    known, parsed = set(rubric_ids()), []
    for finding in findings:
        match = FINDING.match(finding)
        if not match:
            raise SystemExit(f"a finding is R<n>=<path>, not {finding!r}")
        if match.group(1) not in known:
            raise SystemExit(f"{match.group(1)} is not a blocker id in .claude/review-rubric.md")
        parsed.append(match.groups())
    return parsed


def record(cwd, verdict, summary, findings=()):
    head, path = _git(cwd, "rev-parse", "HEAD"), verdict_path(cwd)
    if verdict not in VERDICTS or not head or not path:
        raise SystemExit(f"usage: record {{{'|'.join(VERDICTS)}}} <summary> (inside a git checkout)")
    cited = parse_findings(verdict, findings)
    with open(path, "w", encoding="utf-8") as f:
        json.dump({"head": head, "verdict": verdict, "summary": summary}, f)
    _append_log(cwd, head, verdict, summary, cited)
    return path


def _append_log(cwd, head, verdict, summary, cited):
    """Best effort: the per-HEAD file is the gate, so a lost log line never blocks a PR."""
    try:
        docs = DOCS_PATTERN
        line = {
            "ts": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "branch": _git(cwd, "rev-parse", "--abbrev-ref", "HEAD"),
            "head": head,
            "verdict": verdict,
            "summary": summary,
            "categories": sorted({categorize(name, docs) for name in _changed(cwd)}),
            "findings": [{"rule": rule, "path": p, "category": categorize(p, docs)} for rule, p in cited],
        }
        with open(log_path(cwd), "a", encoding="utf-8") as f:
            f.write(json.dumps(line) + "\n")
    except Exception as exc:
        print(f"warning: the pr-critic log was not written ({exc}); the verdict was", file=sys.stderr)


def tally(entries):
    """Per category and rubric id, how many branches the critic reviewed and blocked."""
    reviewed, blocked, rules, waived = {}, {}, {}, set()
    for entry in entries:
        branch, verdict = entry.get("branch"), entry.get("verdict")
        if verdict == "waived":
            waived.add(branch)
        if verdict not in REVIEWED:
            continue
        for category in entry.get("categories", []):
            reviewed.setdefault(category, set()).add(branch)
        for finding in entry.get("findings", []):
            blocked.setdefault(finding.get("category"), set()).add(branch)
            rules.setdefault(finding.get("rule"), set()).add(branch)
    branches = set().union(*reviewed.values()) if reviewed else set()
    categories = {c: {"reviewed": len(b), "blocked": len(blocked.get(c, ()))} for c, b in sorted(reviewed.items())}
    return {
        "reviewed": len(branches),
        "waived": len(waived),
        "categories": categories,
        "rules": {rule: len(rules.get(rule, ())) for rule in rubric_ids()},
        "propose_exempt": [c for c, n in categories.items()
                           if c not in NEVER_EXEMPT and n["reviewed"] >= PROPOSE_AFTER and not n["blocked"]],
        "propose_prune": ([rule for rule in rubric_ids() if not rules.get(rule)]
                          if len(branches) >= PROPOSE_AFTER else []),
    }


def report(cwd):
    path = log_path(cwd)
    if not path or not os.path.exists(path):
        return "No pr-critic verdicts logged yet."
    with open(path, encoding="utf-8") as f:
        entries = [json.loads(line) for line in f if line.strip()]
    out = tally(entries)
    lines = [f"pr-critic verdicts: {out['reviewed']} branches reviewed, {out['waived']} waived ({path})", "",
             "category: branches reviewed / blocked"]
    lines += [f"  {c}: {n['reviewed']} / {n['blocked']}" for c, n in out["categories"].items()]
    lines += ["", "rubric id: branches whose blocks cited it"]
    lines += [f"  {rule}: {n}" for rule, n in out["rules"].items()]
    lines += [""]
    lines += [f"propose exemption: {c} (reviewed on {out['categories'][c]['reviewed']} branches, never blocked)"
              for c in out["propose_exempt"]]
    lines += [f"propose pruning: {rule} (no block cited it in {out['reviewed']} branches)"
              for rule in out["propose_prune"]]
    if not out["propose_exempt"] and not out["propose_prune"]:
        lines += [f"no proposals: a category or rule needs {PROPOSE_AFTER} branches of evidence"]
    lines += ["", "Proposals are not applied: a human edits the exemption or the rubric, citing this report."]
    return "\n".join(lines)


def docs_only(cwd):
    """True when the branch changes files against origin/main, every one is docs,
    and none of them steers the agents.

    Any failure answers False, so the critic is still required: this exemption
    fails closed, unlike the gate around it.
    """
    changed = _changed(cwd)
    if not changed:
        return False
    return all(DOCS_PATTERN.search(name) and not STEERS_AGENTS.search(name) for name in changed)


def gate_reason(cwd):
    """Why `gh pr create` must wait, or None when the PR may open (or we can't tell)."""
    head, path = _git(cwd, "rev-parse", "HEAD"), verdict_path(cwd)
    if not head or not path:
        return None  # not a checkout we can read: fail open
    if docs_only(cwd):
        return None  # docs-only: the rubric judges code, not prose
    how = (
        "Run the pr-critic agent on this branch (fresh context: give it the spec id or "
        "acceptance criteria and "
        "the base, not the conversation); fix or rebut its blockers, commit, and let it "
        "record a verdict. Agent definitions load when a session starts: if the `pr-critic` "
        "type is not found, or `.claude/agents/pr-critic.md` changed since then, spawn a "
        "general-purpose agent that reads that file and follows it. "
        "Only if the user explicitly skipped review: "
        "`python3 scripts/pr_critic_verdict.py record waived \"<user's words>\"`."
    )
    try:
        with open(path, encoding="utf-8") as f:
            saved = json.load(f)
    except FileNotFoundError:
        return f"No pr-critic verdict for this branch. {how}"
    except Exception:
        return f"The pr-critic verdict file is unreadable. {how}"
    if not isinstance(saved, dict) or saved.get("head") != head:
        return f"The pr-critic verdict is for another commit, not HEAD {head[:8]}. {how}"
    if saved.get("verdict") not in UNLOCKING:
        return f"The pr-critic blocked HEAD {head[:8]}: {saved.get('summary', '')}. {how}"
    return None


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    rec = commands.add_parser("record", help="record a verdict for HEAD")
    rec.add_argument("verdict", choices=VERDICTS)
    rec.add_argument("summary")
    rec.add_argument("--finding", action="append", default=[], metavar="R<n>=<path>",
                     help="a block's citation: rubric id and the path it is in (repeatable)")
    commands.add_parser("report", help="verdict history per category and rubric id, with proposals")
    args = parser.parse_args()
    if args.command == "record":
        print(record("", args.verdict, args.summary, args.finding))
    else:
        print(report(""))
