#!/usr/bin/env python3
"""PreToolUse hook — boundary enforcement for dangerous git / gh commands.

Ported from the Calligence harness, where each rule was an instruction until it
failed:
  - never push directly to main (protected; work lands through a squash-merged
    PR from a feat/<title> branch),
  - never use bare `git stash` / `git stash pop` (the stash stack is shared
    across the main checkout, every worktree, and concurrent sessions),
  - never push a `claude/*` auto-worktree branch (rename to feat/<title>
    first; GitHub cannot re-point a PR's head),
  - merging a PR or changing secrets/workflows changes what main and the
    release builds contain.

Verdicts:
  - force-push (`-f`, `--force*`, `+refspec`) whose destination is main -> "deny"
  - `git push` whose destination resolves to main, or that cannot be
    resolved (a variable or `$(…)` refspec, `--all`, `--mirror`)     -> "ask"
  - `git push` of a claude/* branch (except deleting it remotely)    -> "deny"
  - bare `git stash`, `git stash pop|clear|save`, untagged `push`    -> "deny"
  - `gh pr merge --auto` (main has no required checks, so it merges
    before CI finishes)                                              -> "deny"
  - `gh pr merge`, `gh api repos/O/R/pulls/N/merge` while `gh pr checks`
    does not show both ci.yml test jobs as `pass`                    -> "deny"
  - `gh pr merge` (CI green or unreadable), `gh api …/pulls/N/merge`,
    `gh secret set|delete`, `gh workflow run|enable|disable`         -> "ask"
  - `gh pr create` without a pr-critic `pass`/`waived` verdict for
    HEAD (scripts/pr_critic_verdict.py)                              -> "deny"

Before matching, the command is: stripped of heredoc bodies (data, unless
the command outside bodies, or one of its substitutions, names a shell,
`eval`, `source` or `.`, which makes them commands; `$(…)` in an unquoted-delimiter body is
always analyzed), split on shell operators outside quotes,
expanded through `$(…)`/backtick substitutions, stripped of wrappers
(`bash -c`, `eval`, `command`, `env`, `timeout`, `xargs`, `sudo -u x`,
absolute paths, shell keywords like `if`/`then`/`do`) and tracked through
`cd` and `git -C`, so the branch a bare `git push` resolves is the branch of
the directory the command actually runs in.

Parsing is heuristic and FAILS OPEN: anything it cannot parse is left to the
normal permission system, and a malformed payload produces no output. It is a
tightening, never the only defense. Wired in .claude/settings.json (PreToolUse,
matcher "Bash").
"""

import json
import os
import re
import shlex
import subprocess
import sys

PROTECTED = {"main"}
SHELLS = {"bash", "sh", "zsh", "dash", "ksh"}
# Wrappers that precede the real command, with the flags that take a value.
PREFIX_COMMANDS = {
    "env": (), "command": (), "builtin": (), "exec": (), "nohup": (),
    "time": (), "caffeinate": (), "npx": (), "bunx": (), "stdbuf": ("-i", "-o", "-e"),
    "sudo": ("-u", "-g", "-p", "-C"), "timeout": ("-s", "-k"), "nice": ("-n",),
    "ionice": ("-c", "-n", "-p"), "xargs": ("-n", "-P", "-I", "-d", "-E", "-L", "-s", "-a"),
}
SHELL_KEYWORDS = {"if", "then", "else", "elif", "fi", "do", "done", "while",
                  "until", "for", "case", "esac", "function", "{", "}", "!", "("}
MAX_DEPTH = 4
MAX_COMMAND = 512_000  # beyond this, shlex costs seconds; pathological input

_GIT_OPTS_WITH_VALUE = {"-C", "-c", "--git-dir", "--work-tree", "--namespace", "--exec-path"}
_PUSH_OPTS_WITH_VALUE = {"-o", "--push-option", "--receive-pack", "--exec", "--repo"}
_DURATION = re.compile(r"^\d+(\.\d+)?[smhd]?$")
_ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
_UNRESOLVABLE = re.compile(r"[$`]")

PUSH_REASON = (
    "This push targets '{target}', the protected branch. CLAUDE.md bans "
    "direct pushes: branch (feat/<title>) + PR, squash-merged. Approve only "
    "on an explicit user request (e.g. the repository's very first push)."
)
FORCE_REASON = (
    "This is a force-push to '{target}', the protected branch. It rewrites "
    "shared history and is never part of the workflow (CLAUDE.md, Git). "
    "Push a feat/<title> branch and open a PR instead."
)
UNRESOLVED_REASON = (
    "This push's destination can't be resolved here ({what}), so it may land "
    "on main. Name the branch explicitly, or confirm the target."
)
CLAUDE_BRANCH_REASON = (
    "'{branch}' is an auto-worktree name. Branches on origin follow "
    "feat/<title> (root CLAUDE.md, Git): run "
    "`git branch -m feat/<title>` and push that instead. Deleting a stale "
    "remote claude/* branch (`git push origin --delete …`) is allowed."
)
MERGE_REASON = (
    "Merging a PR changes main, which every release is built from. CI is green; "
    "merge with --squash only once the pr-critic has passed HEAD and the branch "
    "is rebased on main (CLAUDE.md, Git)."
)
API_MERGE_REASON = (
    "This GitHub API call merges a PR — the same change as `gh pr merge`, under "
    "the same conditions (CLAUDE.md, Git)."
)
# The ci.yml matrix jobs that must pass before any merge (CLAUDE.md, Git).
REQUIRED_CI_JOBS = ("test (macos-latest)", "test (windows-latest)")
_MERGE_OPTS_WITH_VALUE = {"-b", "--body", "-F", "--body-file", "-t", "--subject",
                          "-A", "--author-email", "--match-head-commit"}
AUTO_MERGE_REASON = (
    "`gh pr merge --auto` merges at once here: main has no required status "
    "checks (branch protection needs GitHub Pro on this private repo), so "
    "nothing makes it wait for CI. Wait until `gh pr checks <n>` shows both "
    "test jobs as pass, then merge without --auto."
)
CI_RED_REASON = (
    "CI is not green on this PR's head ({status}). CLAUDE.md lets a session "
    "merge only once test (macos-latest) and test (windows-latest) both pass. "
    "Wait for them (`gh pr checks <n> --watch`), fix a failure, then merge."
)
CI_UNKNOWN_REASON = (
    "Couldn't read this PR's CI status (`gh pr checks` failed). Merge only "
    "once test (macos-latest) and test (windows-latest) both pass (CLAUDE.md, Git)."
)
STASH_REASON = (
    "The git stash stack is shared with the main checkout and every other "
    "worktree/session (see CLAUDE.md): bare `git stash`, `pop`, `clear` "
    "and untagged `push` can clobber or drop another session's entry. Use "
    "a temporary WIP commit, or `git stash push -u -m \"<unique-tag>\"` "
    "then `git stash apply <sha>` (never pop) and drop your entry by tag."
)


def _split_segments(command):
    """Split on ; & | && || and newlines that sit OUTSIDE quotes."""
    segments, buf, quote, i, n = [], [], None, 0, len(command)
    while i < n:
        c = command[i]
        if quote:
            buf.append(c)
            if c == "\\" and quote == '"' and i + 1 < n:
                buf.append(command[i + 1])
                i += 2
                continue
            if c == quote:
                quote = None
            i += 1
            continue
        if c in ("'", '"'):
            quote = c
        elif c == "\\" and i + 1 < n:
            buf.append(c + command[i + 1])
            i += 2
            continue
        elif c == "&" and ((buf and buf[-1].endswith(">")) or command[i + 1:i + 2] == ">"):
            pass  # a redirection (2>&1, &>file), not an operator
        elif c in ";|&\n":
            segments.append("".join(buf))
            buf = []
            if c in "|&" and command[i + 1:i + 2] == c:
                i += 1
            i += 1
            continue
        buf.append(c)
        i += 1
    segments.append("".join(buf))
    return [s.strip() for s in segments if s.strip()]


def _substitutions(text):
    """Yield the inside of every $(…) and `…` substitution."""
    out, i, n = [], 0, len(text)
    while i < n:
        if text.startswith("$(", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text[j] == "(":
                    depth += 1
                elif text[j] == ")":
                    depth -= 1
                j += 1
            out.append(text[i + 2:j - 1])
            i = j
        elif text[i] == "`":
            j = text.find("`", i + 1)
            if j == -1:
                break
            out.append(text[i + 1:j])
            i = j + 1
        else:
            i += 1
    return out


def _strip_prefixes(tokens):
    """Drop assignments, subshell/keyword noise and wrapper commands."""
    changed = True
    while tokens and changed:
        changed = False
        head = tokens[0]
        if head in SHELL_KEYWORDS or _ASSIGNMENT.match(head):
            tokens, changed = tokens[1:], True
            continue
        if head.startswith("(") and len(head) > 1:
            tokens, changed = [head[1:]] + tokens[1:], True
            continue
        if head.startswith("$'") and head.endswith("'"):  # ANSI-C quoting
            tokens, changed = [head[2:-1]] + tokens[1:], True
            continue
        base = os.path.basename(head)
        if base in PREFIX_COMMANDS:
            value_flags = PREFIX_COMMANDS[base]
            tokens, changed = tokens[1:], True
            while tokens:
                tok = tokens[0]
                if tok in value_flags:
                    tokens = tokens[2:]
                elif tok.startswith("-") or _DURATION.match(tok):
                    tokens = tokens[1:]
                else:
                    break
    while tokens and tokens[-1] in (")", "}", ";"):
        tokens = tokens[:-1]
    if tokens and len(tokens[-1]) > 1 and tokens[-1][-1] in (")", "}", ";"):
        tokens = tokens[:-1] + [tokens[-1][:-1]]  # `(git push origin main)`
    return tokens


def _parse_git(tokens):
    """Return (subcommand, args, cwd_override) for a `git ...` token list."""
    i, cwd_override = 1, None
    while i < len(tokens):
        tok = tokens[i]
        if tok == "-C" and i + 1 < len(tokens):
            cwd_override = tokens[i + 1]
            i += 2
            continue
        if tok in _GIT_OPTS_WITH_VALUE:
            i += 2
            continue
        if tok.startswith("-"):
            i += 1
            continue
        return tok, tokens[i + 1:], cwd_override
    return None, [], cwd_override


def _current_branch(cwd):
    try:
        out = subprocess.run(
            ["git", "symbolic-ref", "--short", "HEAD"],
            cwd=cwd or None, capture_output=True, text=True, timeout=5,
        )
        return out.stdout.strip() if out.returncode == 0 else None
    except Exception:
        return None


def _normalize_dest(dest):
    for prefix in ("refs/heads/", "heads/"):
        if dest.startswith(prefix):
            return dest[len(prefix):]
    return dest


def _check_push(args, cwd):
    positionals, deleting, push_all, force = [], False, False, False
    i = 0
    while i < len(args):
        tok = args[i]
        if tok in _PUSH_OPTS_WITH_VALUE:
            i += 2
            continue
        if tok in ("-d", "--delete"):
            deleting = True
        if tok in ("--all", "--mirror"):
            push_all = True
        if tok.startswith("--force") or (
            tok.startswith("-") and not tok.startswith("--") and "f" in tok
        ):
            force = True
        if tok.startswith("-"):
            i += 1
            continue
        positionals.append(tok)
        i += 1

    verdicts = []
    if push_all:
        verdicts.append(("ask", UNRESOLVED_REASON.format(what="it pushes every branch")))

    refspecs = positionals[1:]  # positionals[0] is the remote
    dests, unresolvable = [], False
    for spec in refspecs:
        if spec.startswith("+"):
            force = True
        spec = spec.lstrip("+")
        if spec.startswith(":"):
            deleting = True
        dest = spec.split(":", 1)[1] if ":" in spec else spec
        if _UNRESOLVABLE.search(dest):
            unresolvable = True
            continue
        dest = _normalize_dest(dest)
        if dest in ("HEAD", "@"):
            dest = _current_branch(cwd) or ""
        dests.append(dest)
    if not refspecs and not push_all:
        dests = [_current_branch(cwd) or ""]

    if unresolvable and not deleting:
        verdicts.append(("ask", UNRESOLVED_REASON.format(what="the branch name is a variable")))
    hits = sorted(PROTECTED.intersection(dests))
    if hits and force and not deleting:
        verdicts.append(("deny", FORCE_REASON.format(target=", ".join(hits))))
    elif hits:
        verdicts.append(("ask", PUSH_REASON.format(target=", ".join(hits))))
    claude = [d for d in dests if d.startswith("claude/")]
    if claude and not deleting:
        verdicts.append(("deny", CLAUDE_BRANCH_REASON.format(branch=claude[0])))
    return verdicts


def _check_stash(args):
    sub, rest = None, []
    for i, tok in enumerate(args):
        if not tok.startswith("-"):
            sub, rest = tok, args[i + 1:]
            break
    allowed = (
        sub in {"list", "show", "apply", "drop", "branch", "store", "create"}
        or (
            sub == "push"
            and (
                "-m" in rest
                or "--message" in rest
                or any(t.startswith("--message=") for t in rest)
            )
        )
    )
    return [] if allowed else [("deny", STASH_REASON)]


def _pr_critic_gate(cwd):
    """Deny `gh pr create` until the pr-critic passed the current HEAD."""
    try:
        import importlib.util
        root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
        path = os.path.join(root, "scripts", "pr_critic_verdict.py")
        spec = importlib.util.spec_from_file_location("pr_critic_verdict", path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        reason = module.gate_reason(cwd)
    except Exception:
        return []  # fail open
    return [("deny", reason)] if reason else []


def _merge_target(args):
    """`gh pr merge` args -> (PR number/URL/branch or None, --repo value or None)."""
    pr, repo, i = None, None, 0
    while i < len(args):
        tok = args[i]
        if tok in ("-R", "--repo"):
            repo = args[i + 1] if i + 1 < len(args) else None
            i += 2
            continue
        if tok.startswith("--repo="):
            repo = tok.split("=", 1)[1]
        elif tok in _MERGE_OPTS_WITH_VALUE:
            i += 2
            continue
        elif not tok.startswith("-") and pr is None:
            pr = tok
        i += 1
    return pr, repo


def _ci_states(pr, repo, cwd):
    """{check name: bucket} from `gh pr checks`, or None when it can't be read."""
    cmd = ["gh", "pr", "checks"] + ([pr] if pr else []) + ["--json", "name,bucket"]
    if repo:
        cmd += ["--repo", repo]
    try:
        out = subprocess.run(cmd, cwd=cwd or None, capture_output=True, text=True,
                             timeout=20)
        # gh exits 8 while checks are pending but still prints the JSON.
        checks = json.loads(out.stdout)
        return {c["name"]: c["bucket"] for c in checks}
    except Exception:
        return None


def _ci_gate(pr, repo, cwd):
    """Deny a merge unless every REQUIRED_CI_JOBS check is `pass` on the PR head."""
    if pr is not None and _UNRESOLVABLE.search(pr):
        return [("ask", CI_UNKNOWN_REASON)]  # a variable PR number: can't check
    states = _ci_states(pr, repo, cwd)
    if states is None:
        return [("ask", CI_UNKNOWN_REASON)]
    missing = [f"{job}: {states.get(job, 'not reported')}"
               for job in REQUIRED_CI_JOBS if states.get(job) != "pass"]
    return [("deny", CI_RED_REASON.format(status=", ".join(missing)))] if missing else []


def _check_gh(args, cwd=""):
    if not args:
        return []
    sub, rest = args[0], args[1:]
    action = rest[0] if rest else ""
    if "--help" in rest or "-h" in rest:
        return []
    if sub == "pr" and action == "create":
        return _pr_critic_gate(cwd)
    if sub == "pr" and action == "merge":
        if "--disable-auto" in rest[1:]:
            return []  # cancels a pending auto-merge; merges nothing
        if any(t == "--auto" or t.startswith("--auto=") for t in rest[1:]):
            return [("deny", AUTO_MERGE_REASON)]
        pr, repo = _merge_target(rest[1:])
        return _ci_gate(pr, repo, cwd) or [("ask", MERGE_REASON)]
    if sub == "api":
        joined = " ".join(rest)
        match = re.search(r"repos/([^/\s]+/[^/\s]+)/pulls/(\d+)/merge\b", joined)
        if match:
            return _ci_gate(match.group(2), match.group(1), cwd) or [("ask", API_MERGE_REASON)]
        if re.search(r"pulls/\d+/merge\b", joined) or "mergePullRequest" in joined:
            return [("ask", API_MERGE_REASON)]
    if sub == "secret" and action in ("set", "delete", "remove"):
        return [("ask", "GitHub secrets feed the CI and release workflows. Changing "
                        "one changes what every build runs with. Confirm it was requested.")]
    if sub == "workflow" and action in ("run", "enable", "disable"):
        return [("ask", "Workflows here build and publish the installers. Confirm "
                        "the workflow and branch.")]
    return []


def _cd_target(tokens, cwd):
    """`cd x` / `pushd x` -> the directory the rest of the command runs in."""
    if not tokens or os.path.basename(tokens[0]) not in ("cd", "pushd"):
        return None
    args = [t for t in tokens[1:] if not t.startswith("-")]
    if not args:
        return None
    target = os.path.expanduser(args[0])
    return target if os.path.isabs(target) else os.path.normpath(os.path.join(cwd or ".", target))


def _heredoc_delimiters(line, quote):
    """Scan one line -> ([(strip_tabs, word, quoted)], quote state at end of line).

    Only `<<` outside quotes opens a heredoc; `<<<` is a here-string.
    """
    delims, i, n = [], 0, len(line)
    while i < n:
        c = line[i]
        if quote:
            if c == "\\" and quote == '"':
                i += 2
                continue
            if c == quote:
                quote = None
            i += 1
            continue
        if c in ("'", '"'):
            quote = c
        elif c == "\\":
            i += 2
            continue
        elif c == "#" and (i == 0 or line[i - 1] in " \t;|&("):
            break  # a comment: nothing after it opens a heredoc
        elif line.startswith("((", i) or line.startswith("$((", i):
            end = line.find("))", i)  # arithmetic: `1<<2` is a shift
            i = n if end == -1 else end + 2
            continue
        elif line.startswith("$[", i):
            end = line.find("]", i)  # deprecated arithmetic `$[1<<2]`
            i = n if end == -1 else end + 1
            continue
        elif line.startswith("<<<", i):
            i += 3
            continue
        elif line.startswith("<<", i):
            j = i + 2
            strip_tabs = line.startswith("-", j)
            j += strip_tabs
            while j < n and line[j] in " \t":
                j += 1
            word, quoted = [], False
            while j < n and line[j] not in " \t;|&<>()":
                if line[j] in ("'", '"'):
                    quoted = True
                    end = line.find(line[j], j + 1)
                    end = n if end == -1 else end
                    word.append(line[j + 1:end])
                    j = end + 1
                else:
                    if line[j] == "\\":
                        quoted = True
                    else:
                        word.append(line[j])
                    j += 1
            if word:
                delims.append((strip_tabs, "".join(word), quoted))
            i = j
            continue
        i += 1
    return delims, quote


def _feeds_shell(text):
    """True when text names something that runs its input as commands (a shell,
    `eval`, `source`, `.`). Any matching word anywhere counts — `bash <<X`,
    `cat <<X |\\\nbash`, `{ cat <<X; } | sh` — which errs toward analyzing."""
    words = re.split(r"[\s|;&()<>{}\\]+", text)  # `bash<<EOF` has no space
    return any(os.path.basename(w.strip("'\"")) in SHELLS | {"eval", "source", "."}
               for w in words)


def _heredocs(command):
    """Remove heredoc bodies -> (command, [(body, delimiter was quoted)])."""
    if "<<" not in command:
        return command, []
    lines, kept, bodies, quote, i = command.split("\n"), [], [], None, 0
    while i < len(lines):
        line = lines[i]
        kept.append(line)
        delims, quote = _heredoc_delimiters(line, quote)
        i += 1
        # A trailing `\` continues the command line; bodies start after it.
        while delims and not quote and line.endswith("\\") and i < len(lines):
            line = lines[i]
            kept.append(line)
            more, quote = _heredoc_delimiters(line, quote)
            delims += more
            i += 1
        for strip_tabs, word, quoted in delims:
            body = []
            while i < len(lines):
                current = lines[i]
                i += 1
                if (current.lstrip("\t") if strip_tabs else current) == word:
                    break
                body.append(current)
            bodies.append(("\n".join(body), quoted))
    return "\n".join(kept), bodies


def analyze(command, cwd, depth=0, runs=False):
    """Return a list of (decision, reason) for one command string.

    Heredoc bodies are data (a commit message, a Python script) unless the
    command, outside its bodies, names a shell — at the top level or inside
    one of its substitutions (or `runs`:
    it sits in a substitution whose output a shell runs); then they are
    analyzed as commands. An unquoted delimiter still expands `$(…)` and
    backticks in its body, so those are analyzed either way.
    """
    if depth > MAX_DEPTH or not isinstance(command, str):
        return []
    command = command.replace("\x00", "")
    if len(command) > MAX_COMMAND:
        return []
    verdicts, current_cwd = [], cwd
    command, bodies = _heredocs(command)
    outside = command
    for inner in _substitutions(command):
        for wrapped in ("$(" + inner + ")", "`" + inner + "`"):
            if wrapped in outside:  # this occurrence only, not every copy of the text
                outside = outside.replace(wrapped, "", 1)
                break
    # A shell inside a substitution (`out=$(bash <<EOF …)`) runs bodies too;
    # each substitution is checked with its own heredoc bodies removed, so a
    # commit message that mentions bash stays data.
    runs = runs or _feeds_shell(outside) or any(
        _feeds_shell(_heredocs(inner)[0]) for inner in _substitutions(command))
    for body, quoted in bodies:
        for executed in [body] if runs else [] if quoted else _substitutions(body):
            verdicts += analyze(executed, cwd, depth + 1)
    for segment in _split_segments(command):
        for inner in _substitutions(segment):
            verdicts += analyze(inner, current_cwd, depth + 1, runs)
        try:
            tokens = shlex.split(segment)
        except ValueError:
            continue  # unbalanced quotes — fail open
        tokens = _strip_prefixes(tokens)
        if not tokens:
            continue
        moved = _cd_target(tokens, current_cwd)
        if moved:
            current_cwd = moved
            continue
        base = os.path.basename(tokens[0])
        if base in SHELLS:
            for j in range(1, len(tokens) - 1):
                flag = tokens[j]
                if flag.startswith("-") and not flag.startswith("--") and "c" in flag:
                    verdicts += analyze(tokens[j + 1], current_cwd, depth + 1)
                    break
        elif base == "eval":
            verdicts += analyze(" ".join(tokens[1:]), current_cwd, depth + 1)
        elif base == "git":
            sub, args, cwd_override = _parse_git(tokens)
            if sub == "push":
                verdicts += _check_push(args, cwd_override or current_cwd)
            elif sub == "stash":
                verdicts += _check_stash(args)
        elif base == "gh":
            verdicts += _check_gh(tokens[1:], current_cwd)
    return verdicts


def main():
    try:
        data = json.load(sys.stdin)
    except Exception:
        return
    if not isinstance(data, dict) or data.get("tool_name") != "Bash":
        return
    tool_input = data.get("tool_input")
    if not isinstance(tool_input, dict):
        return
    command = tool_input.get("command")
    cwd = data.get("cwd")
    verdicts = analyze(command, cwd if isinstance(cwd, str) else "")
    if not verdicts:
        return
    decision = "deny" if any(d == "deny" for d, _ in verdicts) else "ask"
    reason = " | ".join(dict.fromkeys(r for _, r in verdicts))
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": decision,
            "permissionDecisionReason": reason,
        }
    }))


if __name__ == "__main__":
    main()
