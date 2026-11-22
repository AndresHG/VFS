"""VFS-27 AC-1: the sidecar's name, the worker's version check and the installer size limit."""

import unittest

import sidecar


class SidecarTest(unittest.TestCase):
    def test_sidecar_name_has_triple_and_suffix(self):
        """VFS-27 AC-1: tauri finds `binaries/vfs-extract-<triple>[.exe]`."""
        self.assertEqual(
            sidecar.sidecar_name("x86_64-pc-windows-msvc", "Windows"),
            "vfs-extract-x86_64-pc-windows-msvc.exe",
        )
        self.assertEqual(
            sidecar.sidecar_name("aarch64-apple-darwin", "Darwin"),
            "vfs-extract-aarch64-apple-darwin",
        )

    def test_hello_must_carry_the_app_version(self):
        """VFS-27 AC-1: a worker of another version is refused."""
        ok = '{"version":"0.2.2+p1","background":true,"priority":1}'
        self.assertTrue(sidecar.hello_version_ok(ok, "0.2.2"))
        self.assertFalse(sidecar.hello_version_ok(ok, "0.2.3"))
        self.assertFalse(sidecar.hello_version_ok("", "0.2.2"))
        self.assertFalse(sidecar.hello_version_ok("not json", "0.2.2"))

    def test_installer_limit_is_16_mb(self):
        """VFS-27 AC-1: the NSIS installer stays at or under 16 MB."""
        self.assertTrue(sidecar.installer_size_ok(13 * 1024 * 1024))
        self.assertTrue(sidecar.installer_size_ok(16 * 1024 * 1024))
        self.assertFalse(sidecar.installer_size_ok(16 * 1024 * 1024 + 1))


if __name__ == "__main__":
    unittest.main()
