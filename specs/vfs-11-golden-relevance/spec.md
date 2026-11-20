# VFS-11 — Golden relevance tests in Spanish

- **Status**: implemented (PR #102)
- **Size**: M. Tests only, no behavior change, but it fixes the ranking contract that the next
  changes (sort P2/issue 66, frecency P5/issue 69, query syntax P8/issue 72, schema work)
  must keep.
- **Links**: [VFS-1](../vfs-1-core-search/spec.md) AC-10 (ranking tiers),
  `docs/research/prior-art-proposals.md` P6, GitHub issue 70.

## Problem

Ranking changes can silently move the right file off the first screen: the unit tests in
`rank.rs` and `tests/search.rs` check single rules on tiny archives, not what a real query
on a real-looking archive returns first. Kensaku guards its ranking with a fixed set of
queries over a fixture tree, asserted in CI. VFS needs the same, in Spanish and shaped like
the end user's archive: accents, `ñ`, one-letter typos, the start of a word, words split
between folder and name, years and short words.

## Acceptance scenarios

Tests cite these ids (`VFS-11 AC-<m>`) in their doc comments.

- AC-1 Given the golden archive (about 60 named files and folders in the shapes of the end
  user's archive, plus a 200-photo phone roll and `Thumbs.db` noise) When each golden query
  runs through the `Vfs` facade with no filters Then its expected file or folder is among the
  first 3 results, or first when the case says so (an exact name, a whole folder name, a
  unique word).
- AC-2 Given the golden queries Then they cover, each with several cases: accents and case,
  `ñ` with and without the tilde, a name stored decomposed (NFD), one typo in a word of 6 or
  more letters, the start of a word, words split between folders and name, years, and words
  under 6 and under 3 characters.
- AC-3 When one or more golden queries miss Then `cargo test` fails once, listing every miss
  with its position and the results that came first, and the mean reciprocal rank of the set.
- AC-4 Given the set Then it has at least 40 cases and every expected entry exists in the
  golden archive, so a typo in the set cannot pass as "not found".

## Non-goals

- Speed. The set runs with a 30 s deadline; latency is VFS-1 AC-9 and the bench.
- Filters, sort orders and bookmarks: they get their own tests in their own specs.
- Content search (VFS-2).
- Replacing the set with the owner's real searches: refresh it from them after Fase 2 (the
  proposal's own risk note).

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Aggregate metric (Recall@5 / MRR thresholds, like Kensaku) or per-query check? | Per query: every case must hold. A threshold lets one regression hide behind another improvement. MRR is reported in the failure message only. | session, under the owner's autonomy rule | 2026-11-20 |
| 2 | Top 3 or top 5? | Top 3 (the issue's wording), and first place for cases where anything else would be wrong. | session | 2026-11-20 |
| 3 | When a later change legitimately reorders a case | Edit the case in that change's PR and say why in its body; never delete a case to make a PR pass. | session | 2026-11-20 |
