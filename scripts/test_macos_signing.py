"""Tests for scripts/macos_signing.py (VFS-28 R3, AC-3, D12)."""

import shutil
import sys
import subprocess
import tempfile
import unittest
from pathlib import Path

import macos_signing as ms


class ParsingTest(unittest.TestCase):
    def test_fingerprint_both_spellings(self):
        """VFS-28 R3: the identity is named by its SHA-1, as OpenSSL 3 or LibreSSL print it."""
        hexes = "ab:" * 19 + "cd"
        self.assertEqual(ms.parse_fingerprint(f"SHA1 Fingerprint={hexes}\n"), "AB" * 19 + "CD")
        self.assertEqual(ms.parse_fingerprint(f"sha1 Fingerprint={hexes.upper()}"), "AB" * 19 + "CD")
        with self.assertRaises(SystemExit):
            ms.parse_fingerprint("unable to load certificate")

    def test_untrusted_self_signed_identity_is_found(self):
        """VFS-28 AC-3: `find-identity` without -v lists the untrusted self-signed identity;
        other identities and the repeated "matching identities" block are ignored."""
        sha = "0123456789ABCDEF0123456789ABCDEF01234567"
        other = "F" * 40
        text = f"""
Policy: Code Signing
  Matching identities
  1) {sha} "{ms.COMMON_NAME}" (CSSMERR_TP_NOT_TRUSTED)
  2) {other} "Someone Else"
     2 identities found

  Valid identities only
     0 valid identities found
"""
        self.assertEqual(ms.parse_identities(text), [sha])
        self.assertEqual(ms.parse_identities("0 identities found"), [])

    def test_designated_requirement(self):
        """VFS-28 AC-3: builds A and B are compared by their designated requirement."""
        text = ('Executable=/tmp/probe-a\n# designated => identifier "app.vfs.desktop" and '
                'certificate root = H"0123"\n')
        self.assertEqual(ms.designated_requirement(text),
                         'identifier "app.vfs.desktop" and certificate root = H"0123"')
        self.assertIsNone(ms.designated_requirement("code object is not signed at all"))

    def test_pkcs12_ciphers_macos_can_import(self):
        """VFS-28 R3: OpenSSL 3 is told to write the SHA-1/3DES .p12 `security import` reads;
        LibreSSL already does."""
        self.assertEqual(ms.pkcs12_cipher_args("LibreSSL 3.3.6"), [])
        args = ms.pkcs12_cipher_args("OpenSSL 3.0.13 30 Jan 2024")
        self.assertIn("PBE-SHA1-3DES", args)
        self.assertIn("sha1", args)


@unittest.skipUnless(shutil.which("openssl") and sys.platform != "win32",
                     "needs openssl; the certificate is made on a Mac or Linux")
class CertificateTest(unittest.TestCase):
    def test_certificate_is_a_long_lived_code_signing_leaf(self):
        """VFS-28 R3, D12: valid ≥ 10 years, code signing only, named «VFS Self-Signed Code
        Signing», and the .p12 opens with its password."""
        with tempfile.TemporaryDirectory() as tmp:
            p12, sha1 = ms.make_certificate(tmp, "pw-for-test")
            pem = subprocess.run(
                ["openssl", "pkcs12", "-in", str(p12), "-passin", "pass:pw-for-test", "-nokeys"],
                capture_output=True, text=True, check=True).stdout
            cert = Path(tmp) / "c.pem"
            cert.write_text(pem, encoding="utf-8")
            text = subprocess.run(["openssl", "x509", "-in", str(cert), "-noout", "-text"],
                                  capture_output=True, text=True, check=True).stdout
            self.assertIn("Code Signing", text)
            self.assertIn("CA:FALSE", text)
            self.assertIn(ms.COMMON_NAME, text)
            ten_years = str(10 * 365 * 24 * 3600)
            still_valid = subprocess.run(
                ["openssl", "x509", "-in", str(cert), "-noout", "-checkend", ten_years],
                capture_output=True)
            self.assertEqual(still_valid.returncode, 0, "certificate expires within 10 years")
            fp = subprocess.run(["openssl", "x509", "-in", str(cert), "-noout", "-fingerprint",
                                 "-sha1"], capture_output=True, text=True, check=True).stdout
            self.assertEqual(ms.parse_fingerprint(fp), sha1)


if __name__ == "__main__":
    unittest.main()
