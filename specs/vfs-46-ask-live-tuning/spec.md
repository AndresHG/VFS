# VFS-46 — The ask flow, tuned on the first live runs

- **Status**: implemented (PR pending)
- **Size**: M (one subsystem; it supersedes VFS-28 R6.4's passage order, D7's default tier, D8
  and R7's citation format, and the live run's model tier in AC-12, and VFS-2 R11's passage
  count)
- **Links**: VFS-28 (ask your archive), PRs #218 and #219, `rag-live` runs 45689319159,
  45690948017, 45693384399, 45699428945 and 45700576923

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
  R8). Supersedes VFS-28 D8 (R6 brings Anthropic to the same schema).
- R3. THE OpenAI answer schema SHALL describe a quote as a short span copied character for
  character from the passage's text, without its `[P]` header.
- R3a. THE rewrite prompt SHALL ask for the nouns a document uses for what is asked
  («duración», «vigencia» for «cuánto dura») among the synonyms, and the `found` description
  SHALL say it is false when the answer would have to be guessed or calculated (q19 and q40 of
  the second run), and true for a fact stated in other words (Luna said not found to a deposit
  of «dos mensualidades», q07 of the fourth run).
- R3b. THE answer prompt SHALL ask for citations through the provider's citation mechanism and
  forbid passage numbers, references and repeated quotations in the answer text: Haiku wrote
  «(1-1)» and repeated the quote in the text instead of citing, and Luna wrote «citeP0».
- R6. THE Anthropic answer SHALL use the same JSON answer schema and numbered passages as
  OpenAI's, instead of `search_result` blocks with native citations: Haiku answered without
  any citation in 11 of 50 live questions, which showed as not found. The answer field SHALL
  still stream as it arrives. Supersedes VFS-28 D8 and R7's citation format for Anthropic.
- R7. THE app SHALL answer with the cheap tier by default; the mid tier is the option (owner,
  2026-11-25). Supersedes VFS-28 D7's default.
- R8. THE content search SHALL return up to 6 best passages per document hit, not 3: in a
  12-page document the chunks answering q19 and q23 ranked 4th and 5th, so the ask flow never
  saw them. Supersedes VFS-2 R11's «at most 3 per hit». Cost: every chunk of a hit is already
  scored; three more 32-word snippets per hit, a few hundred bytes, no extra read.
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
- AC-6 (R6) Given an Anthropic answer, Then the request holds the numbered passages and the
  JSON schema, and the answer's text and quotes are parsed from it; the answer field streams
  in pieces.
- AC-7 (R7) Given no saved settings, Then the cheap tier is on.
- AC-8 (R8) Given a long document whose eight chunks each hold the searched word, When it is
  searched, Then its hit carries six passages.
- AC-5 (gate) The golden tests pass offline on the recordings of a cheap live run (Haiku,
  run 45699428945: 50/50 found right, 50/50 facts). The other provider's run is reported, not
  recorded (Luna, run 45700576923: 49/50; q20's rewrite varies between runs and missed page 5
  once).

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
| D2 | Golden fixes | q10, q14 and q18 drop a fact the question does not ask for; q24 checks «cuatro», the count asked (three by concurso plus one by concurso-oposición); the fact check drops a thousands separator between digits, so «450.000» matches «450 000». | agent | 2026-11-25 |
| D3 | Model of the live runs | The cheap tier, plus one reference run with the best model if the owner approves its cost. | owner | 2026-11-25 |
| D4 | Default tier | The cheap tier: Luna answered 48 of 50 right against Opus's 48, at 0.02 $ per 50 questions. | owner | 2026-11-25 |

## Open questions

None blocking.
