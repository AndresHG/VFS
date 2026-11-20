"""Tests for guard-git-danger.py's merge gate: no PR merges before CI is green.

Run: python3 -m unittest discover -s .claude/hooks -p "test_*.py"
`gh pr checks` is stubbed, so the tests need neither gh nor the network.
"""

import importlib.util
import json
import os
import subprocess
import unittest
from unittest import mock

_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "guard-git-danger.py")
_SPEC = importlib.util.spec_from_file_location("guard_git_danger", _PATH)
guard = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(guard)

MAC, WIN = guard.REQUIRED_CI_JOBS


def checks(mac, win, returncode=0):
    """A fake `gh pr checks --json name,bucket` result."""
    payload = json.dumps([{"name": MAC, "bucket": mac}, {"name": WIN, "bucket": win}])
    return subprocess.CompletedProcess([], returncode, stdout=payload, stderr="")


class MergeGateTest(unittest.TestCase):
    def verdict(self, command, gh_result):
        with mock.patch.object(guard.subprocess, "run", return_value=gh_result) as run:
            verdicts = guard.analyze(command, "")
        decision = ("deny" if any(d == "deny" for d, _ in verdicts)
                    else "ask" if verdicts else None)
        return decision, run

    def test_merge_refused_while_checks_pending(self):
        # gh exits 8 while checks are pending; the JSON is still on stdout.
        decision, run = self.verdict("gh pr merge 95 --squash", checks("pass", "pending", 8))
        self.assertEqual(decision, "deny")
        cmd = run.call_args.args[0]
        self.assertEqual(cmd[:4], ["gh", "pr", "checks", "95"])

    def test_merge_refused_when_a_job_failed(self):
        decision, _ = self.verdict("gh pr merge 95 --squash", checks("fail", "pass", 1))
        self.assertEqual(decision, "deny")

    def test_merge_refused_when_a_job_is_missing(self):
        only_mac = subprocess.CompletedProcess(
            [], 0, stdout=json.dumps([{"name": MAC, "bucket": "pass"}]), stderr="")
        decision, _ = self.verdict("gh pr merge 95 --squash", only_mac)
        self.assertEqual(decision, "deny")

    def test_merge_asks_once_both_jobs_pass(self):
        decision, _ = self.verdict("gh pr merge 95 --squash", checks("pass", "pass"))
        self.assertEqual(decision, "ask")

    def test_auto_merge_always_refused(self):
        decision, run = self.verdict("gh pr merge 95 --squash --auto", checks("pass", "pass"))
        self.assertEqual(decision, "deny")
        run.assert_not_called()

    def test_auto_with_value_refused(self):
        decision, _ = self.verdict("gh pr merge 95 --auto=true", checks("pass", "pass"))
        self.assertEqual(decision, "deny")

    def test_variable_pr_asks_without_claiming_green(self):
        with mock.patch.object(guard.subprocess, "run") as run:
            verdicts = guard.analyze("gh pr merge $N --squash", "")
        run.assert_not_called()
        self.assertEqual(verdicts, [("ask", guard.CI_UNKNOWN_REASON)])

    def test_disable_auto_is_not_a_merge(self):
        decision, _ = self.verdict("gh pr merge 95 --disable-auto", checks("pending", "pending"))
        self.assertIsNone(decision)

    def test_unreadable_ci_status_still_asks(self):
        broken = subprocess.CompletedProcess([], 1, stdout="", stderr="no network")
        decision, _ = self.verdict("gh pr merge 95 --squash", broken)
        self.assertEqual(decision, "ask")

    def test_repo_flag_and_value_options_are_passed_through(self):
        _, run = self.verdict(
            'gh pr merge --subject "x y" -R AndresHG/VFS 95 --squash', checks("pass", "pass"))
        cmd = run.call_args.args[0]
        self.assertEqual(cmd[3], "95")
        self.assertEqual(cmd[-2:], ["--repo", "AndresHG/VFS"])

    def test_merge_inside_a_compound_command_is_gated(self):
        decision, _ = self.verdict(
            "cd /tmp && gh pr merge 95 --squash --delete-branch", checks("pending", "pass", 8))
        self.assertEqual(decision, "deny")

    def test_api_merge_refused_while_checks_pending(self):
        decision, run = self.verdict(
            "gh api -X PUT repos/AndresHG/VFS/pulls/95/merge -f merge_method=squash",
            checks("pending", "pending", 8))
        self.assertEqual(decision, "deny")
        cmd = run.call_args.args[0]
        self.assertEqual(cmd[3], "95")
        self.assertEqual(cmd[-2:], ["--repo", "AndresHG/VFS"])


if __name__ == "__main__":
    unittest.main()
