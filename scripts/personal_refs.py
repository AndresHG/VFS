#!/usr/bin/env python3
"""Public-repo hygiene: committed text never names the authors' relatives or private life.

VFS is open source; anyone can read every tracked file. Docs, specs, fixtures and mockups
describe users generically ("the user", "they") and use invented sample data. This check
flags first-person kinship phrases ("my mother", "mi padre") and colloquial parent words
("mamá", "mom") in tracked text files.

Standard library only. `python3 scripts/personal_refs.py` exits 1 and lists each hit as
path:line; CI runs it on every PR.
"""

import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parents[1]

_KIN_EN = (
    r"mother|mom|mum|father|dad|wife|husband|sons?|daughters?|kids|"
    r"brothers?|sisters?|grand(?:ma|pa|mother|father|parents?)|family|aunt|uncle|cousins?|"
    r"girlfriend|boyfriend"
)
_KIN_ES = (
    r"madre|padres?|mam[áa]|pap[áa]|mujer|marido|espos[oa]|hij[oa]s?|herman[oa]s?|"
    r"abuel[oa]s?|familia|t[íi][oa]s?|prim[oa]s?|novi[oa]"
)
PATTERNS = [
    re.compile(rf"\b(?:my|our)\s+(?:{_KIN_EN})\b", re.IGNORECASE),
    re.compile(rf"\b(?:mis?|nuestr[oa]s?)\s+(?:{_KIN_ES})\b", re.IGNORECASE),
    # Bare "dad" is left out: "Dad clic" is ordinary Spanish UI copy.
    re.compile(r"\b(?:mamá|papá|mom|mum)\b", re.IGNORECASE),
]

# This file and its test hold the patterns themselves.
SKIP = {"scripts/personal_refs.py", "scripts/test_personal_refs.py"}


def find_hits(text):
    """(line_number, line) for every line that matches a pattern."""
    return [
        (n, line.strip())
        for n, line in enumerate(text.splitlines(), 1)
        if any(p.search(line) for p in PATTERNS)
    ]


def tracked_files(repo=REPO):
    out = subprocess.run(
        ["git", "ls-files", "-z"], cwd=repo, capture_output=True, check=True
    ).stdout
    return [p for p in out.decode("utf-8").split("\0") if p and p not in SKIP]


def main():
    failures = []
    for rel in tracked_files():
        try:
            text = (REPO / rel).read_text(encoding="utf-8")
        except (UnicodeDecodeError, FileNotFoundError, IsADirectoryError):
            continue  # binary, or deleted in the working tree
        failures += [f"{rel}:{n}: {line}" for n, line in find_hits(text)]
    if failures:
        print("Personal references in committed text (use generic wording):")
        print("\n".join(failures))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
