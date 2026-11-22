# VFS-13 — A suggestion when a search finds nothing

- **Status**: approved
- **Size**: M. One subsystem (the empty result) plus one button on screen. It adds a core call
  (`Vfs::suggest`), a Tauri command and its contract types; no index schema change.
- **Links**: [VFS-1](../vfs-1-core-search/spec.md) S5, S7, S8, AC-9;
  [VFS-11](../vfs-11-golden-relevance/spec.md) (golden set, must stay green);
  `docs/research/prior-art-proposals.md` P9; GitHub issue 73 (design notes in its comments).

## Problem

"No se encontró nada" is a dead end for a person who does not know why. Three causes are
common and cheap to detect: a filter hides the file, a short word is misspelt (terms under
`FUZZY_MIN_LEN`, 6 letters, get no typo tolerance), or one word of several is not in the name
or path. Today the screen can only say "check what you typed" or offer to clear every
filter, without knowing whether that would find anything.

## Contract

`Vfs::suggest(&SearchQuery) -> Result<Option<Suggestion>>`; Tauri command `suggest` with the
same query the screen searched (files only). `Suggestion` (camelCase in JSON):

- `kind`: `dropFilters` | `spelling` | `dropTerm`.
- `text`: the search-box text to run, built by the core from the typed text (every typed
  word that folds to the term replaced or removed in place, the rest untouched); unchanged
  for `dropFilters`.
- `clearFilters`: true only for `dropFilters` (type, date, size and folder filters, and the
  scope back to «Todo» per VFS-38 R13; the sources and the bookmark folder stay as they
  are).
- `term`: the typed word that changes (`spelling`, `dropTerm`); `replacement`: the new word
  (`spelling`).
- `count`: hits of the suggested search as the list would show them (grouped copies, VFS-19),
  at most the hit cap; `capped`: the cap was reached.

## Requirements (EARS)

- R1. WHEN a search that includes files ends with zero hits and was not cut by the deadline,
  THE screen SHALL ask the core for one suggestion for that query's text and filters (tries
  count in relevance order whatever order is on screen: the order changes no match).
- R2. WHEN asked, THE core SHALL try, in this order, and return the first try that finds at
  least one hit:
  - (a) only when the text is not empty and a filter is set: the same text without filters;
  - (b) for each term of 4 or 5 characters, all letters (no digit or punctuation), in typed
    order, keeping filters: first its adjacent transpositions, then (5-letter terms only) its single-letter
    deletions; the first variant that finds something wins (generation order, not count);
  - (c) for a query of two or more terms, keeping filters: the query without one term,
    trying terms from last to first; the first that finds something wins.
- R3. Every try counts hits only: no page of results is resolved, and the S7 cache of the
  last ranked list is left as it was.
- R4. THE core SHALL spend at most 1.5 s per call (`SearchConfig::suggest_budget`): each
  try's deadline (soft and hard) is the budget left; a try cut by it counts as finding
  nothing, and no try starts once the budget is spent. A newer search interrupts the call,
  which then returns no suggestion (a search generation and a count of searches waiting for
  the lock, checked between tries and by the running try's progress handler, so an
  interrupt that reached no statement is not lost). `suggest`
  never interrupts a search: it waits for a running one, then runs.
- R5. WHEN a suggestion exists and no other search started since the empty one (the screen's
  sequence number), THE screen SHALL show one button with the suggestion's copy and count;
  pressing it SHALL set the search box to `text`, clear the filters when `clearFilters`, and
  search.
- R6. IF there is no suggestion, or another search started meanwhile, THEN THE screen SHALL
  show today's empty state unchanged.

## Acceptance scenarios

Tests cite these ids (`VFS-13 AC-<m>`).

- AC-1 (R2a) Given "factura" with the type filter Fotos, no photo named factura, but
  documents that match When a suggestion is asked Then it is `dropFilters` with the
  unfiltered count; with no filter set, or with empty text, try (a) is skipped.
- AC-2 (R2b) Given "bdoa" (no match), files named "boda…" and a file named "Lisboa" When a
  suggestion is asked Then it is `spelling` "bdoa" → "boda" (a transposition wins before any
  deletion); a term of 3 letters, of 6 or more, or with a digit or punctuation ("ab.cd")
  gets no spelling try.
- AC-3 (R2c) Given "boda 2019" with no file holding both words, files with "boda" and a file
  "boda 2018" When a suggestion is asked Then it is `dropTerm` "2019" with the count of
  "boda", and `text` is the typed text without that word.
- AC-4 (R2, R6) Given a query for which no try finds anything When a suggestion is asked Then
  there is none.
- AC-5 (R4) Given a budget too short for the tries When a suggestion is asked Then it returns
  within the budget plus 100 ms with no suggestion or an earlier try's; a newer search on the
  same `Vfs` makes it return none; a running search is never cut by a suggestion.
- AC-6 (R3) Given a cached ranked list for a query When a suggestion runs for another query
  Then the next page of the first query comes from the same list.
- AC-7 (R5) The copy is «Ningún resultado con estos filtros. ¿Buscar sin filtros? (N)»,
  «¿Quería decir «boda»? (N)» and «¿Buscar sin «2019»? (N)», with N as «más de 2.000» when
  capped; proven by the node screen tests and the fake backend, and the command by the
  Tauri contract test.

## Non-goals

- Substitution or insertion typos for short words; long words already tolerate one edit.
- Deletions of 4-letter terms: their 3-letter variants match far too broadly.
- Suggestions when a search finds something, more than one suggestion, or more tries.
- Bookmarks: a suggestion counts files only; a search of bookmarks alone gets none.
- Learning from what the person picks.

## Pre-mortem

- Slow HDD: tries run only on zero hits, count only, within 1.5 s each call and interrupted by
  typing; the costly tries are variants that match broadly, bounded by the search caps and
  the budget.
- Long paths, NFC/NFD, antivirus, power cut, permissions, cloud placeholders: n/a because
  suggestions are read-only queries over the index, like any search.
- A million matches: counts reuse the hit cap and say «más de 2.000».
- User files: never opened or touched.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| 1 | Which tries, in which order | Filters, then short-word spelling, then dropping a word (issue 73 design notes) | owner (issue 73) | 2026-11-20 |
| 2 | Budget | 1.5 s per suggestion, only on zero hits | owner (issue 73) | 2026-11-20 |
| 3 | Which variant or dropped word wins | The first that finds something in a fixed order (transpositions, then deletions; last word first), not the most hits: a shorter variant matches more by substring and would win for the wrong reason | session, after spec review | 2026-11-21 |
| 4 | Approval of this spec | An independent critic reviewing only the spec (owner's standing instruction) | owner | 2026-11-21 |
