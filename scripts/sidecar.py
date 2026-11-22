#!/usr/bin/env python3
"""The extraction worker as a Tauri sidecar (VFS-27 R1, PR 27a).

    sidecar.py build              # build vfs-extract (size profile) and place it for tauri
    sidecar.py check --bundle DIR # after `tauri build`: worker present, right version, size

`build` runs `cargo build -p vfs-content --bin vfs-extract --profile extract --locked`
(VFS-2 D16) and copies the result to `apps/desktop/src-tauri/binaries/vfs-extract-<host
triple>[.exe]`, the name `tauri.sidecar.conf.json`'s `externalBin` expects. Release builds pass
that config with `--config`, so local and test builds need no worker.

`check` looks in the bundle folder `tauri build` wrote (`target/release/bundle`):
- macOS: `VFS.app/Contents/MacOS/vfs-extract` sits next to the app's executable;
- Windows: the NSIS installer is at most 16 MB and lists `vfs-extract.exe`; the copy the
  bundler took (`target/release/vfs-extract.exe`) is checked like the macOS one;
and in both cases the worker starts and says hello with the app's version (VFS-2 R17).
"""

import argparse
import json
import platform
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TAURI = ROOT / "apps" / "desktop" / "src-tauri"
BINARIES = TAURI / "binaries"
WORKER = "vfs-extract"

# VFS-27 AC-1: the Windows installer with the worker inside stays at or under 16 MB.
MAX_INSTALLER_BYTES = 16 * 1024 * 1024


def exe_suffix(system=None):
    return ".exe" if (system or platform.system()) == "Windows" else ""


def host_triple():
    out = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        if line.startswith("host: "):
            return line.split(": ", 1)[1].strip()
    raise SystemExit("rustc -vV printed no host triple")


def sidecar_name(triple, system=None):
    """The file name tauri expects for `externalBin: binaries/vfs-extract`."""
    return f"{WORKER}-{triple}{exe_suffix(system)}"


def app_version():
    return json.loads((TAURI / "tauri.conf.json").read_text(encoding="utf-8"))["version"]


def hello_version_ok(hello_line, version):
    """The worker's hello carries the crate version plus a protocol suffix (`0.2.2+p1`); it
    must be the app's version."""
    try:
        reported = json.loads(hello_line)["version"]
    except (ValueError, KeyError, TypeError):
        return False
    return reported.split("+", 1)[0] == version


def installer_size_ok(size):
    return size <= MAX_INSTALLER_BYTES


def build():
    subprocess.run(
        ["cargo", "build", "-p", "vfs-content", "--bin", WORKER, "--profile", "extract", "--locked"],
        cwd=ROOT,
        check=True,
    )
    src = ROOT / "target" / "extract" / f"{WORKER}{exe_suffix()}"
    BINARIES.mkdir(parents=True, exist_ok=True)
    dst = BINARIES / sidecar_name(host_triple())
    shutil.copy2(src, dst)
    print(f"{dst.relative_to(ROOT)}: {dst.stat().st_size} bytes")


def say_hello(worker):
    """Starts the worker, reads its first line, and lets it exit by closing its stdin."""
    p = subprocess.Popen([str(worker)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    try:
        line = p.stdout.readline()
    finally:
        p.stdin.close()
        p.wait(timeout=30)
    return line


def check_worker(worker, version, errors):
    if not worker.is_file():
        errors.append(f"missing worker: {worker}")
        return
    line = say_hello(worker)
    if not hello_version_ok(line, version):
        errors.append(f"{worker}: hello {line.strip()!r} is not version {version}")
    else:
        print(f"{worker}: {line.strip()}")


def check(bundle):
    bundle = Path(bundle)
    version = app_version()
    errors = []
    if platform.system() == "Darwin":
        apps = list((bundle / "macos").glob("*.app"))
        if len(apps) != 1:
            raise SystemExit(f"expected one .app in {bundle / 'macos'}, found {apps}")
        macos = apps[0] / "Contents" / "MacOS"
        check_worker(macos / WORKER, version, errors)
    elif platform.system() == "Windows":
        installers = list((bundle / "nsis").glob("*-setup.exe"))
        if len(installers) != 1:
            raise SystemExit(f"expected one installer in {bundle / 'nsis'}, found {installers}")
        size = installers[0].stat().st_size
        if not installer_size_ok(size):
            errors.append(f"installer is {size} bytes, over {MAX_INSTALLER_BYTES}")
        print(f"{installers[0].name}: {size} bytes")
        listing = subprocess.run(
            ["7z", "l", str(installers[0])], capture_output=True, text=True, check=True
        ).stdout
        if f"{WORKER}.exe" not in listing:
            errors.append(f"{installers[0].name} does not contain {WORKER}.exe")
        check_worker(bundle.parent / f"{WORKER}.exe", version, errors)
    else:
        raise SystemExit("bundles are built on macOS and Windows only")
    if errors:
        raise SystemExit("\n".join(errors))


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("build")
    c = sub.add_parser("check")
    c.add_argument("--bundle", default=str(ROOT / "target" / "release" / "bundle"))
    a = ap.parse_args()
    if a.cmd == "build":
        build()
    else:
        check(a.bundle)


if __name__ == "__main__":
    sys.exit(main())
