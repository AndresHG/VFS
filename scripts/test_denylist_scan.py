"""Tests for denylist_scan.py: denylisted text and non-noreply commit emails are caught, and
a hit never echoes the matched text.

Run: python3 -m unittest discover -s scripts -p "test_*.py"
"""

import contextlib
import importlib.util
import io
import os
import pathlib
import subprocess
import tempfile
import unittest
from unittest import mock

_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "denylist_scan.py")
_SPEC = importlib.util.spec_from_file_location("denylist_scan", _PATH)
scan = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(scan)

# Invented sample entries; the real list never enters the repo.
LIST = "# comment\n\nZorblat Ltd\nre:qx-\\d{4}\n"


class ParseAndFindTest(unittest.TestCase):
    def test_substring_is_case_insensitive_and_regex_entries_work(self):
        pats = scan.parse(LIST)
        self.assertEqual(len(pats), 2)
        self.assertEqual(scan.find(pats, "ok\nworks at zorblat ltd\nid QX-1234"), [(2, 1), (3, 2)])

    def test_clean_text_has_no_hits(self):
        self.assertEqual(scan.find(scan.parse(LIST), "Zorblat\nqx-12"), [])

    def test_env_wins_over_file_and_missing_file_means_empty(self):
        with tempfile.TemporaryDirectory() as d:
            f = pathlib.Path(d, "list")
            f.write_text("from-file\n", encoding="utf-8")
            self.assertEqual(len(scan.load({"VFS_DENYLIST": "a\nb", "VFS_DENYLIST_FILE": str(f)})), 2)
            self.assertEqual(len(scan.load({"VFS_DENYLIST_FILE": str(f)})), 1)
            self.assertEqual(scan.load({"VFS_DENYLIST_FILE": str(f) + ".missing"}), [])


class DiffTest(unittest.TestCase):
    DIFF = (
        "diff --git a/docs/a.md b/docs/a.md\n--- a/docs/a.md\n+++ b/docs/a.md\n"
        "@@ -3,0 +4,2 @@\n+fine line\n+I work at Zorblat Ltd\n"
        "diff --git a/zorblat ltd.txt b/zorblat ltd.txt\n--- /dev/null\n+++ b/zorblat ltd.txt\n"
        "@@ -0,0 +1 @@\n+x\n"
        "diff --git a/old.md b/old.md\n--- a/old.md\n+++ /dev/null\n@@ -1 +0,0 @@\n-Zorblat Ltd\n"
    )

    def test_added_lines_and_paths_are_flagged_by_location_and_entry_only(self):
        msgs = scan.scan_diff(scan.parse(LIST), self.DIFF)
        self.assertEqual(
            msgs,
            ["docs/a.md:5: matches denylist entry #1", "changed file #2: file path matches denylist entry #1"],
        )
        # Removing a denylisted line is never a hit, and the matched text never appears,
        # not even as a file path.
        self.assertFalse(any("zorblat" in m.lower() or "work at" in m for m in msgs))


class RangeTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = self.tmp.name
        self.git("init", "-q", "-b", "main")
        self.commit("base.txt", "base\n", "base", "1+u@users.noreply.github.com")
        self.base = self.git("rev-parse", "HEAD").strip()

    def tearDown(self):
        self.tmp.cleanup()

    def git(self, *args, env=None):
        return subprocess.run(
            ["git", *args], cwd=self.repo, check=True, capture_output=True, text=True,
            env={**os.environ, **(env or {})},
        ).stdout

    def commit(self, name, text, msg, email):
        pathlib.Path(self.repo, name).write_text(text, encoding="utf-8")
        self.git("add", name)
        who = {"GIT_AUTHOR_NAME": "u", "GIT_AUTHOR_EMAIL": email,
               "GIT_COMMITTER_NAME": "u", "GIT_COMMITTER_EMAIL": email}
        self.git("commit", "-q", "-m", msg, env=who)

    def run_range(self, patterns):
        cwd = os.getcwd()
        os.chdir(self.repo)
        try:
            return scan.scan_range(patterns, self.base)
        finally:
            os.chdir(cwd)

    def test_private_commit_email_fails_even_without_a_denylist(self):
        self.commit("a.txt", "a\n", "feat: a", "someone@example.com")
        msgs = self.run_range([])
        self.assertEqual(len(msgs), 2)
        self.assertTrue(all("email is not a noreply address" in m for m in msgs))
        self.assertFalse(any("example.com" in m for m in msgs))

    def test_noreply_commits_with_clean_content_pass(self):
        self.commit("a.txt", "a\n", "feat: a", "25706859+u@users.noreply.github.com")
        self.assertEqual(self.run_range(scan.parse(LIST)), [])

    def test_denylisted_message_and_content_fail(self):
        self.commit("a.txt", "id qx-9876\n", "feat: for Zorblat Ltd", "noreply@anthropic.com")
        msgs = self.run_range(scan.parse(LIST))
        self.assertEqual(len(msgs), 2)
        self.assertIn("message line 1: matches denylist entry #1", msgs[0])
        self.assertEqual(msgs[1], "a.txt:1: matches denylist entry #2")


class MainTest(unittest.TestCase):
    def test_text_mode_exits_1_on_a_hit_and_0_when_clean(self):
        with mock.patch.dict(os.environ, {"VFS_DENYLIST": LIST}):
            for body, code in (("PR body mentions Zorblat Ltd", 1), ("clean body", 0)):
                with self.subTest(body=body), mock.patch("sys.stdin", io.StringIO(body)):
                    out = io.StringIO()
                    with contextlib.redirect_stdout(out):
                        self.assertEqual(scan.main(["text"]), code)
                    self.assertNotIn("Zorblat", out.getvalue())


if __name__ == "__main__":
    unittest.main()
