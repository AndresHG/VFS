# Future lines: order, shared pieces and a possible later phase

Companion to [future-lines.md](future-lines.md), which holds the research, costs, sources and
open questions for each line. This page orders the lines, names what they share, and proposes
how the ones worth doing could become one coherent later phase. It records the owner's
decisions of 2026-11-20; nothing else here is scheduled. The deciding question for each open
line is in the summary table of [future-lines.md](future-lines.md#summary).

## Product invariant

**VFS is purely consultative.** It never operates on, modifies or writes to any source: her
files, Firefox, YouTube or her chats. Sources are read in place or through an export she
makes herself. What VFS produces (an exported list, a spreadsheet) is a new file written where
she chooses. Every line below respects this; the one that could not (reorganising bookmarks)
is not done.

## Decided

| Line | Decision | Reason |
|---|---|---|
| [5. Open-internet search](future-lines.md#line-5) ([#81](https://github.com/AndresHG/VFS/issues/81), closed) | Will not do | Any LLM or agent already searches the web better than VFS could. |
| [7. Reorganising bookmarks](future-lines.md#line-7) ([#83](https://github.com/AndresHG/VFS/issues/83), closed) | Will not do for now | Applying a new structure writes to Firefox, which breaks the invariant. |
| [2. Unnamed photos by place, date or content](future-lines.md#line-2) ([#78](https://github.com/AndresHG/VFS/issues/78)) | Out of scope; kept as a future line | Photos are found by name and folder. The section keeps the costs in case the owner ever revisits it. |

## Order of the open lines

The order weighs value for this user against cost and risk on the target PC. Value is judged
from her six example requests: four of them (1, 2, 3, 5) are thematic searches over
documents. Cost covers build effort, disk and CPU time on a slow PC with one busy HDD, money,
and how much she has to do herself. Risk covers privacy, terms of service and the chance of
damaging her data.

| # | Line | Value for her | Cost | Risk | Verdict |
|---|---|---|---|---|---|
| 1 | [6. Dates and source sorting](future-lines.md#line-6) ([#82](https://github.com/AndresHG/VFS/issues/82)) | Medium alone; asked for directly, and the base for 3, 4 and 8 | Small: the dates are already on every hit; an in-memory sort of at most 2,000 hits | Low; the danger is presenting an authoring date as a publication date | **Do next.** Implementation-ready; files now, bookmarks with VFS-6 |
| 2 | [1. Thematic gathering](future-lines.md#line-1) ([#77](https://github.com/AndresHG/VFS/issues/77)) | Highest: 4 of her 6 examples | Low on top of phase 3 (stemming, collections); query expansion costs fractions of a cent per search **[estimate]** | Low; the expansion step sends only the query, opt-in | **Do next**, inside phase 3 and then phase 5 |
| 3 | [3. YouTube via Takeout](future-lines.md#line-3) ([#79](https://github.com/AndresHG/VFS/issues/79)) | Medium; depends on whether she uses named playlists | Small importer; she requests a Takeout export once in a while | Low; Takeout data is hers and is outside the YouTube API storage rules | Do next after VFS-6 |
| 4 | [8. Bibliographic list](future-lines.md#line-8) ([#84](https://github.com/AndresHG/VFS/issues/84)) | Medium-high for her studies; an explicit request | Small once phase 3 extraction exists; online lookups are free | Low; lookups send ISBNs and DOIs only | Do later, right after phase 3 |
| 5 | [4. Telegram and WhatsApp](future-lines.md#line-4) ([#80](https://github.com/AndresHG/VFS/issues/80)) | Medium; some of what she wants may live in chats | Medium: two parsers, locale-dependent formats; she exports chat by chat | Medium: other people's messages; unofficial routes break terms | Do later, exports only |

Line 6 comes first, ahead of the higher-value line 1, because it is small and fully specified,
and the owner judged it reasonable. Line 1's first step sits inside phase 3, which is already
under way.

Never do, whatever the order: face recognition (biometric data, GDPR article 9); the Telegram
API on her account (risk of a ban); decrypting WhatsApp databases (against WhatsApp's terms);
any write to a source, Firefox's `places.sqlite` included; local text embeddings on the target
PC before #41 measures them there.

## What the open lines share

### A common "source" abstraction

VFS-6 makes bookmarks the first source besides files, with their own tables in a separate
database file and a source selector. Lines 3, 4 and 8 add more sources or item kinds: YouTube
videos, chat attachments, links and contacts, books. They need one item model so that one
search, one result list and one sort cover them all:

- `source` (files, bookmarks, YouTube, Telegram, WhatsApp) and `kind` (file, folder, link,
  video, photo, contact, book);
- a locator: a path to reveal in Explorer/Finder, or a URL to open;
- a title and searchable text, fed to the same accent-folded name index and, where there is
  text, to the phase 3 full-text index;
- `published_at` and `collected_at`, each with where it came from and a confidence
  ([line 6](future-lines.md#line-6));
- a link from an item to its container (a playlist, a chat, a bookmark folder, an import
  run), so an import can be removed as a whole.

The import pattern is the same for YouTube, Telegram and WhatsApp: she (or the owner) puts an
official export in a folder, VFS reads it in the isolated worker and records the import run,
and a re-import replaces that run. VFS only reads the export; it never writes back to the
service. This keeps every source offline after import and avoids live accounts. VFS-6 should
leave room for this model without building it: a `source` on each hit and a normalised URL on
each bookmark are enough today. Line 6's first step adds the per-hit source label.

### Content extraction

The phase 3 extraction worker (issue #23), an isolated process with time and memory limits,
is the place for every parser that reads untrusted files: document text (lines 1 and 8), EPUB
OPF and PDF metadata (line 8), Takeout CSV, `result.json` and `_chat.txt` (lines 3 and 4).
Phase 3's queueing (one reader on the HDD, yield to the name indexer, skip cloud-only
placeholders as in #49) applies unchanged. Lines 1 and 8 are cheap only because phase 3 builds
this.

### Optional model access

Phase 5 (#38) defines the optional AI module: off by default, the owner's own Anthropic or
OpenAI key in the OS credential store, and a preview of what leaves the PC before anything is
sent. Among the open lines only line 1 needs it, for query expansion, which sends only the
search words (#40). At current prices that costs fractions of a cent per search
**[estimate]**. Its real cost is privacy, so the consent preview matters more than the price.

### Network access under an offline promise

Some open lines need the internet even without a model: oEmbed titles for YouTube IDs (line
3), Open Library and Crossref lookups (line 8). Each is small and read-only, but together they
erode "fully offline". They should share one rule, designed once with #38's consent screen:
each network feature is off until switched on, says in Spanish what it sends and to whom, and
can be switched off again.

### Dates and the result list

Lines 3, 4 and 8 all end in a list sorted by a date that is not the file's modification time.
Line 6's sort selector is the first piece; its two-date model is the second. «Colecciones»,
saved searches with pins and exclusions from line 1, are the natural container for her «reúne
todo» requests and for the bibliographic export of line 8.

## A possible later phase

1. **Inside work already planned, no new phase.** The sort selector and source label of line
   6 go in now, files first and bookmarks with VFS-6 (#63). Spanish stemming and a test set
   from her six sentences go into phase 3 (#24). LLM query expansion is the first concrete
   feature of phase 5 (#38, #40).

2. **«Fase 6 · Fuentes y colecciones»**, one coherent phase: the common source model, the
   import pattern, the two-date model and «Colecciones», then the YouTube Takeout importer and
   the bibliographic list, then the Telegram and WhatsApp importers. It is all read-only and
   offline after import, needs no account integration, and reuses the phase 3 worker. It
   answers her request for one command over all her sources, sortable by date and source,
   without any AI.

Optional online extras (live YouTube sync through OAuth with a read-only scope, Open Library
and Crossref lookups) attach to phase 6 behind the shared network-access rule, each when she
asks for it. Photo content stays out unless the owner revisits line 2.
