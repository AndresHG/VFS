#!/usr/bin/env python3
"""The embedding worker as a release asset (VFS-29 R16, D21).

    embed_worker.py build --out DIR    # build vfs-embed with ONNX Runtime, name, sign, check

The worker is never in the installer (VFS-27 AC-1's 16 MB budget, D18): the app downloads it
with the model when search by meaning is turned on, from this version's release, and checks
its minisign signature against the updater key. `build`:
- runs `cargo build -p vfs-semantic --features embedder --bin vfs-embed --release --locked`;
- copies it to `DIR/vfs-embed-<os>-<arch>[.exe]`, the asset name the app asks for
  (`vfs_semantic::install::worker_asset`: Rust's `std::env::consts::{OS, ARCH}`);
- on macOS, code-signs it with `APPLE_SIGNING_IDENTITY` (ad hoc when unset);
- signs it with the updater key (`TAURI_SIGNING_PRIVATE_KEY`, through the Tauri CLI), which
  writes `<asset>.sig`;
- checks the worker starts and says hello with the app's version (its fake-model test hook).
"""

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TAURI = ROOT / "apps" / "desktop" / "src-tauri"
TAURI_CLI = os.environ.get("TAURI_CLI_VERSION", "2.12.6")

# Python's names for the platform, as Rust's std::env::consts spells them.
OS_NAMES = {"Darwin": "macos", "Windows": "windows", "Linux": "linux"}
ARCH_NAMES = {"arm64": "aarch64", "aarch64": "aarch64", "AMD64": "x86_64", "x86_64": "x86_64"}


def asset_name(system=None, machine=None):
    """`vfs-embed-<os>-<arch>[.exe]` for this platform, as the app looks for it."""
    system = system or platform.system()
    machine = machine or platform.machine()
    suffix = ".exe" if system == "Windows" else ""
    return f"vfs-embed-{OS_NAMES[system]}-{ARCH_NAMES[machine]}{suffix}"


def app_version():
    return json.loads((TAURI / "tauri.conf.json").read_text(encoding="utf-8"))["version"]


def hello_ok(line, version):
    """The worker's hello carries the crate version plus a protocol suffix (`0.4.0+e1`)."""
    try:
        hello = json.loads(line)
    except ValueError:
        return False
    return str(hello.get("version", "")).split("+")[0] == version


def build(out):
    subprocess.run(
        ["cargo", "build", "-p", "vfs-semantic", "--features", "embedder", "--bin", "vfs-embed",
         "--release", "--locked"],
        cwd=ROOT, check=True)
    exe = ".exe" if platform.system() == "Windows" else ""
    out.mkdir(parents=True, exist_ok=True)
    dest = out / asset_name()
    shutil.copy2(ROOT / "target" / "release" / f"vfs-embed{exe}", dest)
    if platform.system() == "Darwin":
        identity = os.environ.get("APPLE_SIGNING_IDENTITY") or "-"
        subprocess.run(["codesign", "--force", "--options", "runtime", "--sign", identity, str(dest)],
                       check=True)
    if not os.environ.get("TAURI_SIGNING_PRIVATE_KEY"):
        sys.exit("TAURI_SIGNING_PRIVATE_KEY is not set: the app refuses an unsigned worker")
    npx = "npx.cmd" if platform.system() == "Windows" else "npx"
    subprocess.run([npx, "--yes", f"@tauri-apps/cli@{TAURI_CLI}", "signer", "sign", str(dest)],
                   check=True)
    if not Path(f"{dest}.sig").is_file():
        sys.exit(f"no signature written for {dest.name}")
    p = subprocess.run([str(dest), "--model-dir", ".", "--test-hook=fake:8"], input="",
                       capture_output=True, text=True, timeout=60)
    line = p.stdout.splitlines()[0] if p.stdout else ""
    if not hello_ok(line, app_version()):
        sys.exit(f"the worker did not say hello with version {app_version()}: {line!r} {p.stderr}")
    print(f"{dest.name}: {dest.stat().st_size} bytes, signed")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build")
    b.add_argument("--out", required=True, type=Path)
    a = ap.parse_args()
    if a.cmd == "build":
        build(a.out)


if __name__ == "__main__":
    main()
