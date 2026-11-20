#!/usr/bin/env bash
# PostToolUse hook — nested CLAUDE.md maintenance nudge (non-blocking).
#
# When an Edit/Write/MultiEdit touches a source file in a directory that has
# its own CLAUDE.md, remind Claude to keep that CLAUDE.md current. It only
# nudges — it never blocks the edit and never modifies anything.
#
# Wired in .claude/settings.json (PostToolUse, matcher "Edit|Write|MultiEdit").
# To disable: remove that hook entry, or delete this file.

input="$(cat)"

# Extract the edited file path from the tool input JSON (stdin).
file_path="$(printf '%s' "$input" | python3 -c 'import sys, json
try: d = json.load(sys.stdin)
except Exception: d = {}
ti = d.get("tool_input") or {}
print(ti.get("file_path") or "")' 2>/dev/null)"

# Nothing to do without a path.
[ -n "$file_path" ] || exit 0

# Never nudge when the edited file IS a CLAUDE.md.
case "$file_path" in
  */CLAUDE.md|CLAUDE.md) exit 0 ;;
esac

# Only the source types these docs actually describe.
case "$file_path" in
  *.rs|*.ts|*.tsx|*.js|*.html|*.css) ;;
  *) exit 0 ;;
esac

# Walk up from the file's directory to the NEAREST CLAUDE.md, stopping at
# (and excluding) the project root — the root CLAUDE.md is maintained
# separately, so edits whose nearest doc is the root stay silent.
root="${CLAUDE_PROJECT_DIR:-}"
doc=""
d="$(dirname "$file_path")"
while [ -n "$d" ] && [ "$d" != "/" ] && [ "$d" != "." ]; do
  [ "$d" = "$root" ] && break
  if [ -f "$d/CLAUDE.md" ]; then
    doc="$d/CLAUDE.md"
    break
  fi
  d="$(dirname "$d")"
done
[ -n "$doc" ] || exit 0

# Tidy relative path for the message.
rel="$doc"
[ -n "$root" ] && rel="${doc#"$root"/}"

# Emit a non-blocking reminder as PostToolUse additionalContext.
python3 -c 'import sys, json
rel, base = sys.argv[1], sys.argv[2]
msg = (f"Note: you edited {base} in a directory that has its own {rel}. "
       f"If this change made a rule there WRONG, edit that rule in place or "
       f"delete it — replace, never append (root CLAUDE.md). What changed "
       f"belongs in the PR body; intent for a change goes in its spec. If no "
       f"rule there is now wrong, change nothing.")
print(json.dumps({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": msg}}))' "$rel" "$(basename "$file_path")"
exit 0
