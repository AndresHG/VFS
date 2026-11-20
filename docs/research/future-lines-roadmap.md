# Future lines: order, shared pieces and a possible later phase

Companion to [future-lines.md](future-lines.md), which holds the research, costs, sources and
open questions for each line. This page orders the eight lines, names what they share, and
proposes how the ones worth doing could become one coherent later phase. Nothing here is
scheduled: the owner decides, and the deciding question for each line is in the summary table
of [future-lines.md](future-lines.md#summary).

## Order

The order weighs value for this user against cost and risk on the target PC. Value is judged
from her six example requests: four of them (1, 2, 3, 5) are thematic searches over
documents, two (4, 6) are about unnamed photos. Cost covers build effort, disk and CPU time
on a slow PC with one busy HDD, money, and how much she has to do herself. Risk covers privacy,
terms of service and the chance of damaging her data.

| # | Line | Value for her | Cost | Risk | Verdict |
|---|---|---|---|---|---|
| 1 | [1. Thematic gathering](future-lines.md#line-1) ([#77](https://github.com/AndresHG/VFS/issues/77)) | Highest: 4 of her 6 examples | Low on top of phase 3 (stemming, collections); query expansion costs fractions of a cent per search **[estimate]** | Low; the expansion step sends only the query, opt-in | Do next |
| 2 | [2. Unnamed photos, metadata first](future-lines.md#line-2) ([#78](https://github.com/AndresHG/VFS/issues/78)) | High: examples 4 and 6, impossible otherwise | One EXIF read per photo, about 1.3–2.5 h of disk time once for 300,000 photos **[estimate]**; a thumbnail grid in the UI | Low for metadata; content models carry cost and privacy trade-offs | Do next **if the owner reopens scope** |
| 3 | [6. Dates and source sorting](future-lines.md#line-6) ([#82](https://github.com/AndresHG/VFS/issues/82)) | Medium alone; the enabler for 2, 3, 4 and 8 | Small: the dates already exist for files and bookmarks | Low; the danger is presenting an authoring date as a publication date | Do next, inside VFS-6 |
| 4 | [5. Open internet, browser button](future-lines.md#line-5) ([#81](https://github.com/AndresHG/VFS/issues/81)) | Medium | Tiny for the button; a Brave API key costs about $0 at 10–30 searches a day **[estimate]** | Button: none. API: her health queries leave the PC | Button next; API later, opt-in |
| 5 | [3. YouTube via Takeout](future-lines.md#line-3) ([#79](https://github.com/AndresHG/VFS/issues/79)) | Medium; depends on whether she uses named playlists | Small importer; she requests a Takeout export once in a while | Low; Takeout data is hers and is outside the YouTube API storage rules | Do next after VFS-6 |
| 6 | [8. Bibliographic list](future-lines.md#line-8) ([#84](https://github.com/AndresHG/VFS/issues/84)) | Medium-high for her studies; an explicit request | Small once phase 3 extraction exists; online lookups are free | Low; lookups send ISBNs and DOIs only | Do later, right after phase 3 |
| 7 | [7. Reorganising bookmarks](future-lines.md#line-7) ([#83](https://github.com/AndresHG/VFS/issues/83)) | Medium; an explicit request, but the mess also hurts search less once VFS-6 searches them | Report: small. Real moves need a signed Firefox extension | Medium-high: anything that writes to Firefox can lose her bookmarks | Do later: report first, moves only with an extension |
| 8 | [4. Telegram and WhatsApp](future-lines.md#line-4) ([#80](https://github.com/AndresHG/VFS/issues/80)) | Medium; example 6 may live in a chat | Medium: two parsers, locale-dependent formats; she exports chat by chat | Medium: other people's messages; unofficial routes break terms | Do later, exports only |

Two lines are deliberately split. Line 2 has a cheap, private first step (metadata) and an
expensive, optional second one (image–text embeddings or captions), and only the first is
ranked here. Line 5 has a free button and an opt-in API, and only the button is ranked high.

What not to do at all, whatever the order: face recognition (biometric data, GDPR article 9);
the Telegram API on her account (risk of a ban); decrypting WhatsApp databases (against
WhatsApp's terms); writing to Firefox's `places.sqlite`; sending a JSON bookmark restore for
her to apply alone; local text embeddings on the target PC before #41 measures them there.

## What the lines share

### A common "source" abstraction

VFS-6 makes bookmarks the first source besides files, with their own tables in a separate
database file and a source selector. Lines 3, 4, 5 and 8 add more sources or item kinds:
YouTube videos, chat attachments, links and contacts, web results, books. They need one item
model so that one search, one result list and one sort cover them all:

- `source` (files, bookmarks, YouTube, Telegram, WhatsApp, web) and `kind` (file, folder,
  link, video, photo, contact, book);
- a locator: a path to reveal in Explorer/Finder, or a URL to open;
- a title and searchable text, fed to the same accent-folded name index and, where there is
  text, to the phase 3 full-text index;
- `published_at` and `collected_at`, each with where it came from and a confidence
  ([line 6](future-lines.md#line-6));
- a link from an item to its container (a playlist, a chat, a bookmark folder, an import
  run), so an import can be removed as a whole.

The import pattern is the same for YouTube, Telegram and WhatsApp: she (or the owner) puts an
official export in a folder, VFS parses it in the isolated worker and records the import run,
and a re-import replaces that run. This keeps every source offline after import and
avoids live accounts. VFS-6 should leave room for this model without building it: a `source`
column and a normalised URL on each bookmark are enough today.

### Content extraction

The phase 3 extraction worker (issue #23), an isolated process with time and memory limits,
is the place for every parser that reads untrusted files: document text (lines 1 and 8), EXIF
and HEIC metadata (line 2), EPUB OPF and PDF metadata (line 8), Takeout CSV, `result.json` and
`_chat.txt` (lines 3 and 4). Phase 3's queueing (one reader on the HDD, yield to the name
indexer, skip cloud-only placeholders as in #49) applies unchanged. Lines 1, 2 and 8 are cheap
only because phase 3 builds this.

### Optional model access

Phase 5 (#38) defines the optional AI module: off by default, the owner's own Anthropic or
OpenAI key in the OS credential store, and a preview of what leaves the PC before anything is
sent. Four lines use it, each sending little:

- line 1: query expansion, which sends only the search words (#40);
- line 2: captions for a subset of photos already filtered by place and date (option D);
- line 7: classifying bookmark titles and URLs into the scheme she states;
- line 5: web search, if done through the provider's search tool instead of Brave.

At current prices each of these costs cents, not dollars, for her volumes **[estimate]**.
Their real cost is privacy, so the consent preview matters more than the price.

### Network access under an offline promise

Several lines need the internet even without a model: oEmbed titles for YouTube IDs, Open
Library and Crossref lookups, dead-link checks, a search API. Each is small, but together they
erode "fully offline". They should share one rule: each network feature is off until switched
on, says in Spanish what it sends and to whom, and can be switched off again. That rule can be
designed once, with #38's consent screen.

### Dates and the result list

Lines 2, 3, 4 and 8 all end in a list sorted by a date that is not the file's modification
time. Line 6's two-date model and a result list that can show a source column, a date column,
and (for photos) a thumbnail grid are shared UI work. «Colecciones», saved searches with pins
and exclusions from line 1, are the natural container for her "reúne todo" requests and
for the bibliographic export of line 8.

## A possible later phase

The lines fall into three groups that could become milestones after phases 3 and 5:

1. **Inside work already planned, no new phase.** Spanish stemming and a test set from her six
   sentences go into phase 3 (#24). The collected-date sort, the source column and normalised
   URLs go into VFS-6 (#63). The «Buscar también en internet» button is an S-sized change at any
   time. LLM query expansion is the first concrete feature of phase 5 (#38, #40).

2. **«Fase 6 · Fuentes y colecciones»**, one coherent phase: the common source model, the
   import pattern, the two-date model, «Colecciones», then the YouTube Takeout importer, the
   bibliographic list and the bookmark report, then the Telegram and WhatsApp importers. All of
   it is offline after import, needs no account integration, and reuses the phase 3 worker. It
   answers her "one command over all my sources, sortable by date and source" request
   without any AI.

3. **Photos, only if the owner reopens scope.** Metadata first (EXIF dates, GPS, place names,
   events, a thumbnail grid): it fits as a step of phase 6 or its own small phase. Image–text
   embeddings computed on the owner's Mac, or captions through phase 5, come only after the
   metadata step's coverage report and a test on about 20 of her photos show they find more.

Optional online extras (a search API, live YouTube sync, Open Library lookups, dead-link
checks, bookmark moves through a signed extension) attach to phase 6 behind the shared
network-access rule, each when she asks for it.
