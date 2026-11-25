#!/usr/bin/env python3
"""The stable self-signed macOS signing identity (VFS-28 R3, AC-3, D6, D12; PR 28a).

    macos_signing.py new-cert --out DIR       # once, on the maintainer's Mac: the certificate
    macos_signing.py import --keychain FILE   # CI: a throwaway keychain holding the identity
    macos_signing.py ab-check --keychain FILE --identity SHA1   # CI: AC-3
    macos_signing.py check-app APP            # CI: the bundle is signed by it
    macos_signing.py cleanup --keychain FILE

An ad-hoc signature is a hash of one build, so a keychain item one release saved is denied
to the next (D6). Every release is instead signed with one self-signed code-signing
certificate, valid for 20 years. Its private key lives in the Actions secret
`MACOS_SIGNING_CERTIFICATE` (a base64 .p12) with `MACOS_SIGNING_CERTIFICATE_PASSWORD`, and in
an offline backup the maintainer keeps (`docs/updates.md`). No Apple account is involved: the
app is not notarized, as before.

`import` reads the .p12 from those two variables (or `--p12` and `--password-file`), puts
it in a new keychain with a random password, lets `codesign` use the key without a prompt,
adds the keychain to the search list, and prints the identity's SHA-1, which Tauri gets as
`APPLE_SIGNING_IDENTITY`. With `--ephemeral` it makes a throwaway certificate first, so pull
requests prove the mechanism without the real secret.

`ab-check` builds `crates/vfs-ai/examples/keychain_probe.rs` twice (builds A and B, two code
hashes), signs both with the identity and the app's identifier, and checks that B reads,
with no prompt allowed, the item A saved through the app's own `OsStore`. A third copy,
signed ad hoc, must be refused: that shows the keychain really checks the signature, so a
pass is not vacuous. A prompt cannot be answered on a runner; each call has a time limit, so
a prompt fails the check instead of hanging it. The item is deleted at the end.
"""

import argparse
import base64
import os
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The certificate's subject. A self-signed signature has no Team ID ("not set"); the
# designated requirement names the certificate itself, which is what keeps it stable.
COMMON_NAME = "VFS Self-Signed Code Signing"
# R3: at least 10 years.
VALID_DAYS = 7305
# The app's bundle identifier (tauri.conf.json), so the probes' designated requirement has
# the app's shape: identifier plus certificate.
IDENTIFIER = "app.vfs.desktop"
P12_ENV = "MACOS_SIGNING_CERTIFICATE"
PASSWORD_ENV = "MACOS_SIGNING_CERTIFICATE_PASSWORD"
PROBE = "keychain_probe"
# Seconds a probe call may take; a keychain prompt waits for ever on a runner.
CALL_TIMEOUT = 30

OPENSSL_CONFIG = f"""[req]
distinguished_name = dn
x509_extensions = ext
prompt = no

[dn]
CN = {COMMON_NAME}
O = VFS

[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
"""


def run(cmd, **kw):
    kw.setdefault("check", True)
    return subprocess.run(cmd, **kw)


def out(cmd, **kw):
    return run(cmd, capture_output=True, text=True, **kw).stdout


def pkcs12_cipher_args(openssl_version):
    """OpenSSL 3 writes AES and SHA-256 .p12 files, which macOS's `security import` rejects;
    the SHA-1 and 3DES ones it reads are LibreSSL's default (macOS's own openssl)."""
    if "LibreSSL" in openssl_version:
        return []
    return ["-keypbe", "PBE-SHA1-3DES", "-certpbe", "PBE-SHA1-3DES", "-macalg", "sha1"]


def make_certificate(out_dir, password):
    """A self-signed code-signing certificate and key in `out_dir/vfs-macos-signing.p12`."""
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    p12 = out_dir / "vfs-macos-signing.p12"
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        (tmp / "req.cnf").write_text(OPENSSL_CONFIG, encoding="utf-8")
        (tmp / "pass").write_text(password, encoding="utf-8")
        run(["openssl", "req", "-x509", "-newkey", "rsa:3072", "-sha256", "-nodes",
             "-days", str(VALID_DAYS), "-config", str(tmp / "req.cnf"),
             "-keyout", str(tmp / "key.pem"), "-out", str(tmp / "cert.pem")],
            capture_output=True)
        version = out(["openssl", "version"])
        run(["openssl", "pkcs12", "-export", "-inkey", str(tmp / "key.pem"),
             "-in", str(tmp / "cert.pem"), "-name", COMMON_NAME, "-out", str(p12),
             "-passout", f"file:{tmp / 'pass'}", *pkcs12_cipher_args(version)])
        fingerprint = out(["openssl", "x509", "-noout", "-fingerprint", "-sha1",
                           "-in", str(tmp / "cert.pem")])
    return p12, parse_fingerprint(fingerprint)


def parse_fingerprint(text):
    """`SHA1 Fingerprint=AB:CD:...` (either openssl's spelling) as 40 upper-case hex digits."""
    m = re.search(r"=\s*([0-9A-Fa-f:]{59})", text)
    if not m:
        raise SystemExit(f"no SHA-1 fingerprint in {text!r}")
    return m.group(1).replace(":", "").upper()


def parse_identities(find_identity_output, name=COMMON_NAME):
    """The SHA-1s of the identities called `name` in `security find-identity` output. A
    self-signed certificate is not trusted, so it is listed only without `-v`, marked
    CSSMERR_TP_NOT_TRUSTED; codesign signs with it all the same."""
    found = []
    for line in find_identity_output.splitlines():
        m = re.match(r'\s*\d+\)\s+([0-9A-F]{40})\s+"([^"]*)"', line)
        if m and m.group(2) == name and m.group(1) not in found:
            found.append(m.group(1))
    return found


def designated_requirement(codesign_output):
    """The designated requirement in `codesign -d -r-` output (it prints on stdout or
    stderr depending on the macOS version, so both are passed in)."""
    m = re.search(r"designated => (.+)", codesign_output)
    return m.group(1).strip() if m else None


def cmd_new_cert(args):
    password = secrets.token_urlsafe(24)
    p12, sha1 = make_certificate(args.out, password)
    pw_file = Path(args.out) / "vfs-macos-signing.password.txt"
    pw_file.write_text(password + "\n", encoding="utf-8")
    b64 = Path(args.out) / "vfs-macos-signing.p12.base64.txt"
    b64.write_text(base64.b64encode(p12.read_bytes()).decode("ascii") + "\n", encoding="ascii")
    for f in (p12, pw_file, b64):
        os.chmod(f, 0o600)
    print(f"Certificate «{COMMON_NAME}», SHA-1 {sha1}, {VALID_DAYS} days.")
    print(f"Wrote {p12}, {pw_file} and {b64}.")
    print("Next: back the folder up offline, then set the two Actions secrets:")
    print(f"  gh secret set {P12_ENV} -R AndresHG/VFS < {b64}")
    print(f"  gh secret set {PASSWORD_ENV} -R AndresHG/VFS < {pw_file}")


def search_list():
    return [line.strip().strip('"') for line in
            out(["security", "list-keychains", "-d", "user"]).splitlines() if line.strip()]


def cmd_import(args):
    keychain = str(Path(args.keychain).resolve())
    with tempfile.TemporaryDirectory() as tmp:
        if args.ephemeral:
            password = secrets.token_urlsafe(18)
            p12, _ = make_certificate(tmp, password)
        elif args.p12:
            p12 = Path(args.p12)
            password = Path(args.password_file).read_text(encoding="utf-8").strip()
        else:
            data, password = os.environ.get(P12_ENV, ""), os.environ.get(PASSWORD_ENV, "")
            if not data or not password:
                raise SystemExit(f"secrets {P12_ENV} and {PASSWORD_ENV} must both be set")
            p12 = Path(tmp) / "identity.p12"
            p12.write_bytes(base64.b64decode(data))
        kc_password = secrets.token_urlsafe(18)
        run(["security", "create-keychain", "-p", kc_password, keychain])
        # No auto-lock during a long build.
        run(["security", "set-keychain-settings", "-lut", "21600", keychain])
        run(["security", "unlock-keychain", "-p", kc_password, keychain])
        run(["security", "import", str(p12), "-k", keychain, "-P", password,
             "-f", "pkcs12", "-T", "/usr/bin/codesign"])
    # codesign may use the key without asking.
    run(["security", "set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s",
         "-k", kc_password, keychain], capture_output=True)
    run(["security", "list-keychains", "-d", "user", "-s", keychain,
         *[k for k in search_list() if k != keychain]])
    ids = parse_identities(out(["security", "find-identity", "-p", "codesigning", keychain]))
    if len(ids) != 1:
        raise SystemExit(f"expected one «{COMMON_NAME}» identity in {keychain}, found {ids}")
    print(f"identity {ids[0]} in {keychain}")
    github_output = os.environ.get("GITHUB_OUTPUT")
    if github_output:
        with open(github_output, "a", encoding="utf-8") as f:
            f.write(f"identity={ids[0]}\n")


def sign(binary, identity, keychain):
    if identity == "-":
        run(["codesign", "--force", "--sign", "-", "--identifier", IDENTIFIER, str(binary)])
    else:
        run(["codesign", "--force", "--options", "runtime", "--identifier", IDENTIFIER,
             "--keychain", keychain, "--sign", identity, str(binary)])


def signature_info(binary):
    p = run(["codesign", "-d", "-r-", "--verbose=4", str(binary)], capture_output=True,
            text=True)
    text = p.stdout + p.stderr
    cdhash = re.search(r"CDHash=([0-9a-f]+)", text)
    return {
        "cdhash": cdhash.group(1) if cdhash else None,
        "requirement": designated_requirement(text),
    }


def probe(binary, *args):
    """(exit code, stdout) of one probe call; a time-out (a prompt) counts as a refusal."""
    try:
        p = subprocess.run([str(binary), *args], capture_output=True, text=True,
                           timeout=CALL_TIMEOUT)
    except subprocess.TimeoutExpired:
        print(f"  {binary.name} {args[0]}: timed out (a keychain prompt?)")
        return None, ""
    print(f"  {binary.name} {args[0]}: exit {p.returncode}, {p.stderr.strip()}")
    return p.returncode, p.stdout.strip()


def build_probe(label, dest):
    env = dict(os.environ, VFS_PROBE_BUILD=label)
    run(["cargo", "build", "-p", "vfs-ai", "--example", PROBE, "--locked"], cwd=ROOT, env=env)
    shutil.copy2(ROOT / "target" / "debug" / "examples" / PROBE, dest)


def cmd_ab_check(args):
    keychain = str(Path(args.keychain).resolve())
    errors = []
    previous_default = out(["security", "default-keychain", "-d", "user"]).strip().strip('"')
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        a, b, c = tmp / "probe-a", tmp / "probe-b", tmp / "probe-adhoc"
        build_probe("A", a)
        build_probe("B", b)
        shutil.copy2(a, c)
        sign(a, args.identity, keychain)
        sign(b, args.identity, keychain)
        sign(c, "-", keychain)
        info_a, info_b = signature_info(a), signature_info(b)
        print(f"A: {info_a}\nB: {info_b}")
        if info_a["cdhash"] == info_b["cdhash"]:
            errors.append("builds A and B have the same code hash: not two builds")
        if not info_a["requirement"] or info_a["requirement"] != info_b["requirement"]:
            errors.append("A and B do not share one designated requirement")
        if f'certificate root = H"{args.identity.lower()}"' not in (info_a["requirement"] or ""):
            errors.append("the designated requirement does not name the identity's certificate")
        account = f"vfs-ab-check-{secrets.token_hex(4)}"
        secret = secrets.token_urlsafe(16)
        # The probes' OsStore saves to the default keychain: the throwaway one, unlocked.
        run(["security", "default-keychain", "-d", "user", "-s", keychain])
        try:
            code, _ = probe(a, "set", account, secret)
            if code != 0:
                errors.append("build A could not save the item")
            code, got = probe(b, "get", account)
            if code != 0 or got != secret:
                errors.append("build B could not read the item A saved without a prompt")
            code, got = probe(c, "get", account)
            if code == 0 and got == secret:
                errors.append("the ad-hoc build read the item: the check proves nothing")
        finally:
            probe(a, "delete", account)
            if previous_default:
                run(["security", "default-keychain", "-d", "user", "-s", previous_default])
    if errors:
        raise SystemExit("AC-3 failed:\n  " + "\n  ".join(errors))
    print("AC-3: build B read build A's keychain item without a prompt; ad-hoc was refused")


def cmd_check_app(args):
    """The bundle verifies, and it and every executable in Contents/MacOS (the app and the
    vfs-extract worker) carry the identity, not an ad-hoc signature."""
    app = Path(args.app)
    run(["codesign", "--verify", "--deep", "--strict", str(app)])
    errors = []
    for code in [app, *sorted((app / "Contents" / "MacOS").iterdir())]:
        p = run(["codesign", "-d", "--verbose=4", str(code)], capture_output=True, text=True)
        text = p.stdout + p.stderr
        if f"Authority={COMMON_NAME}" not in text:
            print(text)
            errors.append(f"{code} is not signed by «{COMMON_NAME}»")
        else:
            print(f"{code}: signed by «{COMMON_NAME}»")
    if errors:
        raise SystemExit("\n".join(errors))


def cmd_cleanup(args):
    keychain = str(Path(args.keychain).resolve())
    run(["security", "list-keychains", "-d", "user", "-s",
         *[k for k in search_list() if k != keychain]])
    run(["security", "delete-keychain", keychain], check=False)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("new-cert", help="make the certificate (once, on the maintainer's Mac)")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_new_cert)
    p = sub.add_parser("import", help="a throwaway keychain holding the identity")
    p.add_argument("--keychain", required=True)
    p.add_argument("--ephemeral", action="store_true", help="a throwaway certificate")
    p.add_argument("--p12")
    p.add_argument("--password-file")
    p.set_defaults(func=cmd_import)
    p = sub.add_parser("ab-check", help="VFS-28 AC-3 on this Mac")
    p.add_argument("--keychain", required=True)
    p.add_argument("--identity", required=True)
    p.set_defaults(func=cmd_ab_check)
    p = sub.add_parser("check-app", help="the bundle is signed by the identity")
    p.add_argument("app")
    p.set_defaults(func=cmd_check_app)
    p = sub.add_parser("cleanup", help="remove the throwaway keychain")
    p.add_argument("--keychain", required=True)
    p.set_defaults(func=cmd_cleanup)
    args = parser.parse_args(argv)
    if sys.platform != "darwin" and args.cmd != "new-cert":
        raise SystemExit(f"{args.cmd} runs on macOS only")
    args.func(args)


if __name__ == "__main__":
    main()
