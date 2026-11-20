"""Tests for guard-git-danger.py: the merge gate (no PR merges before CI is green)
and heredoc handling (bodies are data unless a shell runs them).

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


class HeredocTest(unittest.TestCase):
    def decision(self, command):
        with mock.patch.object(guard.subprocess, "run",
                               return_value=checks("pending", "pending", 8)) as run:
            verdicts = guard.analyze(command, "")
        decision = ("deny" if any(d == "deny" for d, _ in verdicts)
                    else "ask" if verdicts else None)
        return decision, run

    def test_data_heredoc_mentioning_git_commands_is_ignored(self):
        decision, run = self.decision(
            "python3 - <<'EOF'\n"
            "msg = 'gh pr merge 95 --auto'\n"
            "git push --force origin main\n"
            "git stash\n"
            "EOF")
        self.assertIsNone(decision)
        run.assert_not_called()

    def test_commit_message_heredoc_in_substitution_is_ignored(self):
        decision, _ = self.decision(
            "git commit -m \"$(cat <<'EOF'\n"
            "fix: the guard\n"
            "gh pr merge 95 --auto\n"
            "git stash pop\n"
            "EOF\n"
            ")\"")
        self.assertIsNone(decision)

    def test_commands_after_the_terminator_are_still_checked(self):
        decision, _ = self.decision("cat > notes.txt <<EOF\nhello\nEOF\ngit stash")
        self.assertEqual(decision, "deny")

    def test_dash_heredoc_ends_at_tab_indented_terminator(self):
        decision, _ = self.decision("cat <<-EOF\n\tgit stash\n\tEOF\ngit stash pop")
        self.assertEqual(decision, "deny")
        decision, _ = self.decision("cat <<-EOF\n\tgit stash\n\tEOF\necho done")
        self.assertIsNone(decision)

    def test_heredoc_fed_to_a_shell_is_analyzed(self):
        decision, _ = self.decision("bash <<'EOF'\ngit push --force origin main\nEOF")
        self.assertEqual(decision, "deny")

    def test_shell_written_against_the_operator_is_analyzed(self):
        for opener in ("bash<<EOF", "zsh<<'EOF'", "cat<<EOF|sh"):
            decision, _ = self.decision(f"{opener}\ngit push --force origin main\nEOF")
            self.assertEqual(decision, "deny", opener)

    def test_shell_later_in_the_command_still_runs_the_body(self):
        for command in (
            "cat <<EOF |\ngit stash pop\nEOF\nbash",
            "cat <<EOF |\\\nbash\ngit stash pop\nEOF",
            "cat <<EOF \\\n| bash\ngit stash pop\nEOF",
            "{ cat <<EOF\ngit stash pop\nEOF\n} | bash",
            "(cat <<EOF\ngit stash pop\nEOF\n) | sh",
            'echo "$(echo "a" <<EOF\ngit stash pop\nEOF\n)" | bash',
        ):
            decision, _ = self.decision(command)
            self.assertEqual(decision, "deny", command)

    def test_heredoc_piped_to_a_shell_is_analyzed(self):
        decision, _ = self.decision("cat <<EOF | sh\ngh pr merge 95 --auto\nEOF")
        self.assertEqual(decision, "deny")

    def test_commit_message_mentioning_a_shell_stays_data(self):
        decision, _ = self.decision(
            "git commit -m \"$(cat <<'EOF'\nrun it with bash or sh\ngit stash pop\nEOF\n)\"")
        self.assertIsNone(decision)

    def test_commit_file_from_stdin_heredoc_is_ignored(self):
        decision, _ = self.decision("git commit -F - <<'EOF'\ngit stash pop\nEOF")
        self.assertIsNone(decision)

    def test_heredoc_substitution_handed_to_eval_is_analyzed(self):
        decision, _ = self.decision('eval "$(cat <<EOF\ngit stash pop\nEOF\n)"')
        self.assertEqual(decision, "deny")

    def test_heredoc_sourced_from_stdin_is_analyzed(self):
        decision, _ = self.decision("source /dev/stdin <<EOF\ngit stash pop\nEOF")
        self.assertEqual(decision, "deny")

    def test_substitutions_in_unquoted_heredoc_body_are_analyzed(self):
        decision, _ = self.decision("cat <<EOF\n$(git push --force origin main)\nEOF")
        self.assertEqual(decision, "deny")
        decision, _ = self.decision("cat <<EOF\n`git stash pop`\nEOF")
        self.assertEqual(decision, "deny")

    def test_substitutions_in_quoted_heredoc_body_stay_data(self):
        for delim in ("'EOF'", '"EOF"', "\\EOF"):
            decision, _ = self.decision(f"cat <<{delim}\n$(git stash pop)\nEOF")
            self.assertIsNone(decision, delim)

    def test_arithmetic_shift_and_comment_are_not_heredocs(self):
        for first in ("echo $((1<<2))", "x=$(( y<<1 ))", "(( z <<= 1 ))", "echo $[1<<2]",
                      "echo hi # a<<b"):
            decision, _ = self.decision(f"{first}\ngit stash pop")
            self.assertEqual(decision, "deny", first)

    def test_shell_heredoc_inside_a_substitution_is_analyzed(self):
        for command in ("out=$(bash <<'EOF'\ngit stash pop\nEOF\n)",
                        "out=$(bash <<EOF\ngit stash pop\nEOF\n)",
                        "x=`bash <<EOF\ngit stash pop\nEOF\n`"):
            decision, _ = self.decision(command)
            self.assertEqual(decision, "deny", command)

    def test_heredoc_in_a_substitution_closed_on_its_line_has_no_body(self):
        for command in ("x=$(cat <<EOF)\ngit stash pop",
                        "echo $(cat <<EOF)\ngit stash pop\nEOF",
                        "echo `cat <<EOF`\necho hi\nEOF\ngit stash pop"):
            decision, _ = self.decision(command)
            self.assertEqual(decision, "deny", command)

    def test_repeated_substitution_text_does_not_hide_the_shell(self):
        decision, _ = self.decision(": $(bash); cat <<EOF | bash $(bash)\ngit stash pop\nEOF")
        self.assertEqual(decision, "deny")

    def test_here_string_is_not_a_heredoc(self):
        decision, _ = self.decision("cat <<< x\ngit stash")
        self.assertEqual(decision, "deny")

    def test_quoted_double_angle_is_not_a_heredoc(self):
        decision, _ = self.decision('echo "<<EOF"\ngit stash')
        self.assertEqual(decision, "deny")


if __name__ == "__main__":
    unittest.main()
