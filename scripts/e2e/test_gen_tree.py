"""Unit tests of the e2e tree generator and WebDriver helpers (VFS-8)."""

import json
import os
import sys
import tempfile
import unicodedata
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gen_tree  # noqa: E402
from wd import WebDriverError, element_id  # noqa: E402


def listing(root):
    out = []
    for d, _, files in os.walk(gen_tree.fs_path(root)):
        out.extend(os.path.join(d, f) for f in files)
    return out


class GenTreeTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.root = os.path.join(cls.tmp.name, "Archivo")
        cls.m = gen_tree.generate(cls.root, files=3000, seed=42)
        cls.files = listing(cls.root)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_count_and_depth(self):
        """VFS-8 AC-8: the tree has the requested file count, about 13 files per folder and
        folders up to 12 levels deep."""
        self.assertEqual(len(self.files), 3000)
        root = gen_tree.fs_path(self.root)
        depths = [os.path.relpath(d, root).count(os.sep) + 1
                  for d, _, _ in os.walk(root) if d != root]
        long_chain = gen_tree.LONG_LEVELS + 1
        self.assertLessEqual(max(depths), max(gen_tree.MAX_DEPTH, long_chain))
        self.assertGreaterEqual(max(depths), 5)
        self.assertEqual(self.m["files"], 3000)
        self.assertGreater(self.m["dirs"], 3000 // 20)

    def test_targets_exist_with_their_shape(self):
        """VFS-8 AC-8: each planted target exists with the size, date and kind its search or
        filter relies on, and its name occurs nowhere else in the tree."""
        names = [unicodedata.normalize("NFC", os.path.basename(f)).lower() for f in self.files]
        for key, t in self.m["targets"].items():
            p = gen_tree.fs_path(t["path"])
            self.assertTrue(os.path.exists(p), key)
            if t["kind"] == gen_tree.KIND_FOLDER:
                self.assertTrue(os.path.isdir(p))
                continue
            st = os.stat(p)
            self.assertEqual(st.st_size, t["size"], key)
            self.assertAlmostEqual(st.st_mtime, t["mtime"], delta=2)
            self.assertEqual(names.count(t["name"].lower()), 1, key)

    def test_spanish_names_and_long_paths(self):
        """VFS-8 AC-8: names carry accents and ñ, some are in NFD, and at least four paths are
        longer than 260 characters."""
        names = [os.path.basename(f) for f in self.files]
        self.assertTrue(any("ñ" in unicodedata.normalize("NFC", n) for n in names))
        nfd = [n for n in names if n != unicodedata.normalize("NFC", n)]
        self.assertGreater(len(nfd), 0.02 * len(names))
        long = [f for f in self.files if len(f.replace("\\\\?\\", "")) > 260]
        self.assertGreaterEqual(len(long), 4)
        self.assertEqual(self.m["paths_over_260"], gen_tree.LONG_EXTRA + 1)

    def test_deterministic(self):
        """VFS-8 AC-8: the same seed gives the same tree."""
        with tempfile.TemporaryDirectory() as t:
            gen_tree.generate(os.path.join(t, "Archivo"), files=3000, seed=42)
            again = sorted(os.path.relpath(f, gen_tree.fs_path(os.path.join(t, "Archivo")))
                           for f in listing(os.path.join(t, "Archivo")))
        first = sorted(os.path.relpath(f, gen_tree.fs_path(self.root)) for f in self.files)
        self.assertEqual(first, again)

    def test_refuses_a_non_empty_root(self):
        """VFS-8 AC-8: the generator never writes into a folder that already has files."""
        with self.assertRaises(SystemExit):
            gen_tree.generate(self.root, files=10)

    def test_manifest_is_json(self):
        """VFS-8 AC-8: the manifest serialises with every key the journey reads."""
        s = json.dumps(self.m, ensure_ascii=False)
        for k in ("root", "targets", "live_name", "live_renamed"):
            self.assertIn(f'"{k}"', s)


class ScanDoneTest(unittest.TestCase):
    def test_scan_done_needs_idle_and_every_root_ready(self):
        """VFS-8 AC-2: the journey waits until the indexer is idle and every root is ready."""
        from journey import ROOT_READY, STATE_IDLE, STATE_SCANNING, scan_done
        ready = {"status": ROOT_READY}
        self.assertTrue(scan_done({"state": STATE_IDLE, "roots": [ready, ready]}))
        self.assertFalse(scan_done({"state": STATE_SCANNING, "roots": [ready]}))
        self.assertFalse(scan_done({"state": STATE_IDLE,
                                    "roots": [ready, {"status": STATE_SCANNING}]}))
        self.assertFalse(scan_done({"state": STATE_IDLE, "roots": []}))


class ExplorerMatchTest(unittest.TestCase):
    TARGET = "C:\\e2e\\Archivo\\Escrituras\\Notaría Valdepeñas escritura 2019.pdf"

    def test_selected_item_is_compared_by_full_path(self):
        """VFS-8 AC-3: the reveal passes when Explorer selects the target, whatever its
        display name (extension hidden) and its path's case or Unicode form."""
        from journey import EXPECT_SELECT, explorer_match
        nfd = unicodedata.normalize("NFD", self.TARGET.upper())
        w = {"folder": "c:\\e2e\\archivo\\escrituras", "selected": [nfd]}
        self.assertIs(explorer_match([w], self.TARGET, EXPECT_SELECT), w)

    def test_display_name_or_other_item_does_not_match(self):
        """VFS-8 AC-3: a window whose selection is another item, or only a display name,
        does not count as the target selected."""
        from journey import EXPECT_SELECT, explorer_match
        folder = "C:\\e2e\\Archivo\\Escrituras"
        for sel in (["Notaría Valdepeñas escritura 2019"],
                    [folder + "\\Notaría Valdepeñas escritura 2019.txt"], []):
            self.assertIsNone(explorer_match([{"folder": folder, "selected": sel}],
                                             self.TARGET, EXPECT_SELECT))

    def test_parent_expectation_needs_a_strict_ancestor(self):
        """VFS-8 AC-3: for a path over 260 characters, an Explorer window on an ancestor of
        the target's folder passes; one on the folder itself or elsewhere does not."""
        from journey import EXPECT_PARENT, explorer_match
        self.assertIsNotNone(explorer_match([{"folder": "C:\\e2e\\Archivo"}],
                                            self.TARGET, EXPECT_PARENT))
        self.assertIsNone(explorer_match([{"folder": "C:\\e2e\\Archivo\\Escrituras"},
                                          {"folder": "D:\\otro"}],
                                         self.TARGET, EXPECT_PARENT))


class WebDriverTest(unittest.TestCase):
    def test_element_reference_by_prefix(self):
        """VFS-8 AC-5: element references are read by the W3C key prefix, so the embedded
        server's misspelt key works as well as the standard one."""
        self.assertEqual(element_id({"element-6066-11e4-a52e-4f735466cecb": "a"}), "a")
        self.assertEqual(element_id({"element-6066-11e4-a52e-4f735466cecf": "b"}), "b")
        with self.assertRaises(WebDriverError):
            element_id({"other": "c"})


if __name__ == "__main__":
    unittest.main()
