# VFS-29b benchmark corpus: sources and licences

The corpus `bench.py build` assembles has three parts.

## Content golden corpus

Every file of `crates/vfs-content/tests/golden` (Spanish Wikipedia excerpts, BOE PDFs and
synthetic household documents), with the sources and licences listed in that folder's
`NOTICE.md`. Its 45 queries are the keyword subset of the gate.

## Wikipedia excerpts (CC BY-SA 4.0): `distractors.jsonl.gz`

Short excerpts (the lead section, at most 3,000 characters) of Spanish and English Wikipedia
articles, unchanged except for truncation at a paragraph or sentence end, fetched from the
MediaWiki API (`prop=extracts`, `exintro`, `explaintext`) by `fetch_corpus.py`. Licence:
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). Authors: the contributors
listed in each article's history.

Each line of the file is one excerpt and carries its own attribution: `title`, `revid` and
`url`, a permanent link to the revision the excerpt was taken from
(`https://<lang>.wikipedia.org/w/index.php?oldid=<revid>`).

## Synthetic documents: `synthetic.py`

The meaning-only targets, the hard negatives and the meaning-only queries were written for
this benchmark; names, figures, addresses and reference numbers are invented. Companies and
public bodies are named only as the issuers such documents usually have.

## Timing sample: `../timing_sample.jsonl.gz`

200 chunks cut from the documents above, as the VFS content index stored them: Wikipedia
excerpts (CC BY-SA 4.0) and golden-corpus documents. Each row's `doc` names its source file in
the assembled corpus; a `dz_<lang>_<n>.txt` file is the n-th row of that language in
`distractors.jsonl.gz`, which carries its title, revision and URL.
