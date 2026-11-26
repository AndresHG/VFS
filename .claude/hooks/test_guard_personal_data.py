"""Tests for guard-personal-data.py: publishing commands carrying denylisted text are denied.

Run: python3 -m unittest discover -s .claude/hooks -p "test_*.py"
"""

import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

_HOOK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "guard-personal-data.py")
_SPEC = importlib.util.spec_from_file_location("guard_personal_data", _HOOK)
guard = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(guard)

LIST = "Zorblat Ltd\n"  # invented sample entry


def run_hook(command, cwd):
    payload = json.dumps({"tool_name": "Bash", "tool_input": {"command": command}, "cwd": cwd})
    out = subprocess.run(
        [sys.executable, _HOOK], input=payload, capture_output=True, text=True,
        env={**os.environ, "VFS_DENYLIST": LIST},
    ).stdout
    return json.loads(out)["hookSpecificOutput"] if out.strip() else None


class GuardTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = self.tmp.name
        subprocess.run(["git", "init", "-q"], cwd=self.dir, check=True)

    def tearDown(self):
        self.tmp.cleanup()

    def test_denies_pr_body_inline_and_from_file_without_echoing_it(self):
        pathlib.Path(self.dir, "body.md").write_text("Built at Zorblat Ltd\n", encoding="utf-8")
        for cmd in ('gh pr create --title t --body "for zorblat ltd"',
                    "gh pr create --title t --body-file body.md",
                    "gh issue comment 5 -F body.md"):
            with self.subTest(cmd=cmd):
                out = run_hook(cmd, self.dir)
                self.assertEqual(out["permissionDecision"], "deny")
                self.assertNotIn("Zorblat", out["permissionDecisionReason"])

    def test_denies_commit_whose_staged_diff_has_denylisted_text(self):
        pathlib.Path(self.dir, "a.md").write_text("Zorblat Ltd\n", encoding="utf-8")
        subprocess.run(["git", "add", "a.md"], cwd=self.dir, check=True)
        out = run_hook('git commit -m "docs: notes"', self.dir)
        self.assertEqual(out["permissionDecision"], "deny")
        self.assertIn("a.md:1", out["permissionDecisionReason"])

    def test_commit_all_checks_unstaged_changes_to_tracked_files(self):
        f = pathlib.Path(self.dir, "a.md")
        f.write_text("clean\n", encoding="utf-8")
        subprocess.run(["git", "add", "a.md"], cwd=self.dir, check=True)
        subprocess.run(["git", "-c", "user.name=u", "-c", "user.email=u@users.noreply.github.com",
                        "commit", "-q", "-m", "init"], cwd=self.dir, check=True)
        f.write_text("clean\nZorblat Ltd\n", encoding="utf-8")
        self.assertIsNone(run_hook('git commit -m "docs: notes"', self.dir))
        out = run_hook('git commit -am "docs: notes"', self.dir)
        self.assertEqual(out["permissionDecision"], "deny")

    def test_allows_clean_and_unrelated_commands(self):
        for cmd in ('gh pr create --title t --body "clean"', "echo Zorblat Ltd", "ls"):
            with self.subTest(cmd=cmd):
                self.assertIsNone(run_hook(cmd, self.dir))


if __name__ == "__main__":
    unittest.main()
