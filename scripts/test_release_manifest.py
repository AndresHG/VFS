"""Tests for release_manifest.py (VFS-21 AC-9). Run: python3 -m unittest scripts/test_release_manifest.py"""

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import release_manifest  # noqa: E402

BASE = "https://github.com/AndresHG/VFS/releases/download/v0.2.0"


class ManifestTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())

    def artifact(self, name, sig="c2lnbmF0dXJl\n"):
        path = self.dir / name
        path.write_bytes(b"payload")
        if sig is not None:
            (self.dir / (name + ".sig")).write_text(sig, encoding="utf-8")
        return path

    def test_entries_carry_signature_content_and_asset_url(self):
        """VFS-21 AC-9: each platform has the .sig content and the release asset URL."""
        win = self.artifact("VFS_0.2.0_x64-setup.exe", "d2lu\n")
        mac = self.artifact("VFS.app.tar.gz", "bWFj")
        out = self.dir / "latest.json"
        code = release_manifest.main([
            "--version", "0.2.0", "--base-url", BASE + "/",
            "--platform", f"windows-x86_64={win}", "--platform", f"darwin-aarch64={mac}",
            "--out", str(out),
        ])
        self.assertEqual(code, 0)
        m = json.loads(out.read_text(encoding="utf-8"))
        self.assertEqual(m["version"], "0.2.0")
        self.assertRegex(m["pub_date"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        self.assertEqual(m["platforms"]["windows-x86_64"], {
            "signature": "d2lu", "url": f"{BASE}/VFS_0.2.0_x64-setup.exe"})
        self.assertEqual(m["platforms"]["darwin-aarch64"], {
            "signature": "bWFj", "url": f"{BASE}/VFS.app.tar.gz"})

    def test_incomplete_input_is_refused(self):
        """VFS-21 AC-9: the app rejects a manifest with any incomplete platform, so the
        script refuses to write one."""
        unsigned = self.artifact("VFS.app.tar.gz", sig=None)
        empty = self.artifact("empty.tar.gz", sig="  \n")
        ok = self.artifact("ok.exe")
        cases = [
            ("0.2.0", {"darwin-aarch64": unsigned}),
            ("0.2.0", {"darwin-aarch64": empty}),
            ("0.2.0", {"darwin-aarch64": self.dir / "missing.tar.gz"}),
            ("0.2.0", {"macos-arm64": ok}),
            ("v0.2", {"windows-x86_64": ok}),
            ("0.2.0", {}),
        ]
        for version, platforms in cases:
            with self.subTest(version=version, platforms=platforms):
                with self.assertRaises(ValueError):
                    release_manifest.build_manifest(version, BASE, platforms, "2026-11-21T00:00:00Z")


if __name__ == "__main__":
    unittest.main()
