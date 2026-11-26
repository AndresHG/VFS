#!/usr/bin/env python3
"""Check that every meaning-only query shares no content keyword with its target (VFS-29 gate).

A content keyword is a word of the query that is not a function word (STOP). Words are
compared as the shipped analyzer indexes them (crates/vfs-content/src/analyze.rs): split
on non-alphanumerics, lowercase, Snowball Spanish stem, then accent fold
(vfs_core::fold_key). English queries are also compared with the Snowball English stem, a
stricter test, since VFS indexes English text with the Spanish stemmer.

    check.py            # exits 1 and lists every shared keyword
"""

import sys
import unicodedata

import snowballstemmer

from synthetic import MEANING, TARGETS

STOP = set("""
a al ante bajo con contra de del desde durante en entre hacia hasta mediante para por segun
sin so sobre tras el la los las lo un una unos unas y e o u ni que como cuando donde cuanto
cuanta cuantos cuantas mi mis tu tus su sus nos nuestro nuestra se me te le les ya muy mas
the of for to in on at by with from and or my our your his her its is are was were be an
s it this that what how
""".split())

ES = snowballstemmer.stemmer("spanish")
EN = snowballstemmer.stemmer("english")


def fold(s):
    s = unicodedata.normalize("NFKD", s.lower())
    return "".join(c for c in s if not unicodedata.combining(c)).lower()


def words(text):
    out, cur = [], []
    for ch in text + " ":
        if ch.isalnum():
            cur.append(ch)
        elif cur:
            out.append("".join(cur))
            cur = []
    return out


def keys(text, stemmer):
    return {fold(stemmer.stemWord(w.lower())) for w in words(text)}


def content_words(query):
    return [w for w in words(query) if len(w) > 1 and fold(w) not in STOP]


def shared(query, doc, lang):
    found = []
    for stemmer in [ES] + ([EN] if lang == "en" else []):
        dk = keys(doc, stemmer)
        for w in content_words(query):
            k = fold(stemmer.stemWord(w.lower()))
            if k in dk:
                found.append(f"{w}->{k}")
    return sorted(set(found))


def main():
    bad = 0
    for q, target, lang in MEANING:
        s = shared(q, TARGETS[target], lang)
        if s:
            bad += 1
            print(f"SHARED «{q}» / {target}: {', '.join(s)}")
    n_es = sum(1 for m in MEANING if m[2] == "es")
    n_en = len(MEANING) - n_es
    print(f"{len(MEANING)} meaning-only queries ({n_es} es, {n_en} en); {bad} share a keyword")
    sys.exit(1 if bad or n_es < 20 or n_en < 8 else 0)


if __name__ == "__main__":
    main()
