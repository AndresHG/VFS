#!/usr/bin/env python3
"""Writes the updater manifest `latest.json` for a release (VFS-21 R9).

The app fetches it from `releases/latest/download/latest.json`; each platform entry names the
release asset to download and carries the content of its minisign `.sig` file, which the app
checks against the public key in tauri.conf.json before installing anything.

Usage:
  release_manifest.py --version 0.2.0 --base-url https://github.com/O/R/releases/download/v0.2.0 \
      --platform windows-x86_64=path/VFS_0.2.0_x64-setup.exe \
      --platform darwin-aarch64=path/VFS.app.tar.gz --out latest.json

Each `--platform KEY=FILE` needs `FILE.sig` next to it; the asset's URL is the base URL plus
the file name.
"""

import argparse
import datetime
import json
import re
import sys
from pathlib import Path
from urllib.parse import quote

PLATFORM_KEY = re.compile(r"^(windows|darwin|linux)-(x86_64|aarch64|i686|armv7)$")
SEMVER = re.compile(r"^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$")


def build_manifest(version, base_url, platforms, pub_date):
    """`platforms` maps a platform key to the updater artifact; returns the manifest dict."""
    if not SEMVER.match(version):
        raise ValueError(f"not a SemVer version: {version}")
    if not platforms:
        raise ValueError("no platform given")
    entries = {}
    for key, artifact in platforms.items():
        if not PLATFORM_KEY.match(key):
            raise ValueError(f"bad platform key: {key}")
        artifact = Path(artifact)
        sig = artifact.with_name(artifact.name + ".sig")
        if not artifact.is_file():
            raise ValueError(f"missing artifact: {artifact}")
        signature = sig.read_text(encoding="utf-8").strip() if sig.is_file() else ""
        if not signature:
            raise ValueError(f"missing or empty signature: {sig}")
        entries[key] = {
            "signature": signature,
            "url": f"{base_url.rstrip('/')}/{quote(artifact.name)}",
        }
    return {
        "version": version,
        "pub_date": pub_date,
        "platforms": entries,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--version", required=True)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--platform", action="append", default=[], metavar="KEY=FILE")
    parser.add_argument("--out", required=True)
    args = parser.parse_args(argv)

    platforms = {}
    for item in args.platform:
        key, sep, path = item.partition("=")
        if not sep:
            parser.error(f"--platform needs KEY=FILE: {item}")
        platforms[key] = path
    pub_date = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    try:
        manifest = build_manifest(args.version, args.base_url, platforms, pub_date)
    except ValueError as e:
        print(f"release_manifest: {e}", file=sys.stderr)
        return 1
    Path(args.out).write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
