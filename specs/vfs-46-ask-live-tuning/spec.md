# VFS-46 — The ask flow, tuned on the first live runs

- **Status**: implemented (PR pending)
- **Size**: M (one subsystem; it supersedes VFS-28 R6.4's passage order, D8's not-found rule for
  OpenAI answers, and the live run's model tier in AC-12)
- **Links**: VFS-28 (ask your archive), PRs #218 and #219, `rag-live` runs 45689319159,
  45690948017 and 45693384399

## Problem

The first live runs of the RAG golden set (VFS-28 AC-12), with OpenAI's cheap tier, judged 40
of 50 questions right on found or not found. Three causes were in the flow, not the model:

- A model that says the passages lack the answer still cites a nearby fact (5 of the 15
  unanswerable questions). VFS-28 D8 detects «not found» as an answer with no valid citation,
  so these showed as answers.
- Quotes that differ from the passage by a character are dropped, and an answer whose
  citations were all dropped shows as not found (3 answerable questions).
- A long document that matched far better than the rest (BM25 22 against 1 to 5) gave one
  passage, and the passages holding the answers went to weak documents (3 questions).

Three golden cases also asked for facts their question does not ask for (q10, q18, q24).

## Requirements (EARS)

- R1. THE passage pick SHALL treat a document scoring under a quarter of the best document's
  score as weak, and SHALL give weak documents passages only after every strong document has
  given all of its own. Within each group the VFS-28 R6.4 order holds. Supersedes VFS-28 R6.4.
- R2. THE OpenAI answer SHALL carry the model's `found` verdict in its schema. WHEN `found` is
  false, THE app SHALL discard the answer's citations, so that it shows as not found (VFS-28
  R8). Supersedes VFS-28 D8 for OpenAI; Anthropic answers keep D8.
- R3. THE OpenAI answer schema SHALL describe a quote as a short span copied character for
  character from the passage's text, without its `[P]` header.
- R3a. THE rewrite prompt SHALL ask for the nouns a document uses for what is asked
  («duración», «vigencia» for «cuánto dura») among the synonyms, and the `found` description
  SHALL say it is false when the answer would have to be guessed or calculated (q19 and q40 of
  the second run).
- R3b. THE answer prompt SHALL ask for citations through the provider's citation mechanism and
  forbid passage numbers, references and repeated quotations in the answer text: Haiku wrote
  «(1-1)» and repeated the quote in the text instead of citing, and Luna wrote «citeP0».
- R4. THE OpenAI calls SHALL send reasoning effort `none`, the least the models take
  (`gpt-6-luna` rejects `minimal` with a 400; VFS-28 D7).
- R5. THE live run SHALL use the cheap tier by default (owner, 2026-11-25), SHALL take an
  optional answer model for one reference run (`claude-opus-5-5`), and its report SHALL list
  each citation the check dropped with the passage it named.

## Acceptance scenarios

Tests cite these ids (`VFS-46 AC-<m>`) in their doc comments.

- AC-1 (R1) Given one document scoring 22 with three passages and ten scoring 5, When passages
  are picked, Then the first three are the strong document's, then the weak ones in order.
- AC-2 (R2) Given an OpenAI answer with `found: false` and citations whose quotes are in their
  passages, When it is parsed, Then it has no citations; the request schema requires `found`.
- AC-3 (R4) Both OpenAI requests send `reasoning.effort` `none`.
- AC-4 (R5) Given an answer request naming `claude-opus-5-5`, Then the request names it, sends
  no `thinking` (Opus 5.5 rejects turning it off) and `output_config.effort` `low`.
- AC-5 (gate) The golden tests pass offline on the recordings of a live run of each provider.

## Non-goals

- A model selector in the app (issue #220).
- Fuzzy quote matching: R3 fixed the drops seen; a quote must still be found verbatim after
  the fold.

## Pre-mortem

- Slow PC / HDD: R1 reorders passages already retrieved; no new read.
- Paths over 260 characters, NFC/NFD, antivirus, kill or power cut, permission-denied, user
  files: n/a because nothing new is opened or written.
- Huge result sets: still at most 10 passages (VFS-28 R6.4).
- Secrets: the live run keeps the keys in repository secrets and never prints them.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Weak share | A quarter of the best score: in the live runs the strong document scored 4 to 20 times the next one, and a quarter keeps documents of similar scores together. | agent | 2026-11-25 |
| D2 | Golden fixes | q10 and q18 drop a fact the question does not ask for; q24 checks «tres» and «concurso-oposición», which any right answer holds, instead of two exact phrasings. | agent | 2026-11-25 |
| D3 | Model of the live runs | The cheap tier, plus one reference run with the best model if the owner approves its cost. | owner | 2026-11-25 |

## Open questions

None blocking.
