"""Tests of scripts/embed_worker.py (VFS-29 D21). Run: python3 -m unittest scripts/test_embed_worker.py"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import embed_worker  # noqa: E402


class AssetName(unittest.TestCase):
    """VFS-29 D21: the asset is named as the app asks for it (Rust's OS and ARCH names)."""

    def test_names_follow_rust(self):
        self.assertEqual(embed_worker.asset_name("Windows", "AMD64"), "vfs-embed-windows-x86_64.exe")
        self.assertEqual(embed_worker.asset_name("Darwin", "arm64"), "vfs-embed-macos-aarch64")
        self.assertEqual(embed_worker.asset_name("Linux", "x86_64"), "vfs-embed-linux-x86_64")


class Hello(unittest.TestCase):
    """VFS-29 R16: a worker from another version is refused before it is published."""

    def test_hello_version(self):
        self.assertTrue(embed_worker.hello_ok('{"version":"0.4.0+e1","dim":8,"background":true}', "0.4.0"))
        self.assertFalse(embed_worker.hello_ok('{"version":"0.3.9+e1","dim":8}', "0.4.0"))
        self.assertFalse(embed_worker.hello_ok("not json", "0.4.0"))


if __name__ == "__main__":
    unittest.main()
