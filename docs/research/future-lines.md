# Future lines of work from the end user's feedback

Status: research, with the owner's decisions of 2026-11-20 recorded per line. Nothing here is
built or scheduled. Each section is written so that a
later spec (`/spec`) can start from it without redoing the investigation. The ordering,
what the lines share and how they could become one later phase are in
[future-lines-roadmap.md](future-lines-roadmap.md). Tracking: milestone
[Líneas futuras](https://github.com/AndresHG/VFS/milestone/7), label `linea-futura`.

## Where this comes from

The end user sent feedback describing what she really wants: one command that searches
several sources at once (her files, about 3,000 Firefox bookmarks, her private YouTube
playlists, and the photos, video links and phone contacts exchanged over Telegram and
WhatsApp), plus the open internet, keeping what she already has apart from what she does not.
She wants results she can sort by publication date, by the date she collected them and by
source. She wants her scattered bookmarks reorganised by a criterion she states, and a
bibliographic list (cover images included) without typing it into Excel. Her example
requests are about themes, not file names:

1. «Necesito estudiar todo lo que tengo sobre hígado graso»
2. «¿Qué recetas usan coco rallado junto con harina de almendra?»
3. «Reúne todo lo que tengo sobre propiedades y fuentes alimentarias del colágeno»
4. «Reúne todas las imágenes que tengo de la cocina de Valle de Ansó» (unnamed photos, in
   folders by year or loose)
5. «Reúne lo que tengo sobre solmisación» (the gatherer must understand the concept)
6. «Todas las fotografías de localizar y quitar la fosa séptica antigua e instalar la nueva,
   por orden cronológico»

The owner decided that, apart from Firefox bookmarks (issue
[#63](https://github.com/AndresHG/VFS/issues/63), spec VFS-6, being built), none of this is
built now. In phase 3 the owner had ruled out the *content* of images, audio and video
([decisions](../decisions.md),
"Scope: names and paths first"). Examples 4 and 6 show that finding unnamed photos matters
to her, so [line 2](#line-2) records what finding them would cost. The owner has kept the
decision: image content stays out of scope, and photos are found by name and folder as today.
Line 2 is kept as a future line with its findings, not as a proposal to reopen it.

## Owner decisions (2026-11-20)

- **Product invariant: VFS is purely consultative.** It never operates on, modifies or writes
  to any source: not her files, not Firefox, not YouTube, not her chats. Sources are read in
  place or through an export she makes herself; anything VFS produces (an exported list, a
  spreadsheet) is a new file written where she chooses, never a change to a source. No line
  below proposes otherwise; where the research found a write path, it is marked as excluded
  by this invariant.
- **[Line 5](#line-5), open-internet search: decided, will not do.** Any LLM or agent already
  searches the web better than VFS could. The research is kept for reference only.
- **[Line 7](#line-7), reorganising bookmarks: decided, will not do for now**, because it
  would write to a source.
- **[Line 2](#line-2), photo content: still out of scope.** Kept as a future line with honest
  costs.
- **[Line 6](#line-6), sorting by dates and by source: reasonable**, the line most likely to be
  done soon. Its section and issue carry implementation-ready acceptance criteria.

Context assumed throughout, from [windows-io.md](windows-io.md) and the
[content search design](../content-search-design.md):
a slow Windows 10/11 PC with one busy 8 TB spinning disk shared with a Synology sync client,
little free memory, an unknown CPU (we budget for an older 4-core x86 with 8 GB of RAM,
possibly without AVX2), fully offline by default, no administrator rights, Spanish UI.

## How figures are marked

- **[published]**: a third-party figure (documentation, pricing page, paper, project), with
  the source named or linked.
- **[measured]**: measured by us, with how. The only measurements in this document are the
  GeoNames lookup in line 2 (dump downloaded 2026-11-20) and one oEmbed call in line 3.
- **[estimate]**: our arithmetic or judgement, with the assumptions stated.

Nothing here was measured on the target PC. Prices and terms were checked against the
providers' own pages in November 2026. They change: re-check them before a spec relies on
them. Where a primary source could not be loaded, the text says so and names the secondary
source used.

## Each section covers

1. What she wants, in her words. 2. How it could work, with two or three options.
3. What it needs from the user (accounts, consent, exports). 4. Privacy and
terms-of-service constraints. 5. Cost on the target PC or in API fees. 6. What it depends on
in VFS. 7. The smallest useful first step. 8. The recommendation. 9. Open product questions
for the owner, the deciding one first. 10. Sources.

## Summary

| Line | Recommendation | Smallest first step | Deciding question for the owner | Issue |
|---|---|---|---|---|
| [1. Thematic gathering](#line-1) | **Do next**: Spanish full text + saved collections (exported as a new list, never written into a source). LLM query expansion later (phase 5). No local embeddings now. | Spanish stemming in the phase 3 full-text index + a test set built from her six sentences; then «Colecciones» | May concept understanding send only the search words (never documents) to an AI service with her key? | [#77](https://github.com/AndresHG/VFS/issues/77) |
| [2. Unnamed photos](#line-2) | **Out of scope (owner decision)**; kept as a future line. If ever revisited: metadata first (EXIF date, GPS, place name, events), embeddings computed on the Mac later, never faces. | (if revisited) Metadata-only photo index plus a coverage report | None now; the decision stands. | [#78](https://github.com/AndresHG/VFS/issues/78) |
| [3. YouTube playlists](#line-3) | **Do next** (after VFS-6): Google Takeout import. OAuth live sync later. | Takeout playlist importer; titles via oEmbed as an optional pass | Does she save into named playlists, or mostly «Ver más tarde» (which the API cannot read)? | [#79](https://github.com/AndresHG/VFS/issues/79) |
| [4. Telegram and WhatsApp](#line-4) | **Do later**, exports only. Never the Telegram API or WhatsApp database decryption. | Telegram `result.json` importer | Which app and which phone hold what she wants (the septic-tank photos, the contacts)? | [#80](https://github.com/AndresHG/VFS/issues/80) |
| [5. Open internet](#line-5) | **Decided: will not do.** Any LLM or agent already searches the web better. | — | — | [#81](https://github.com/AndresHG/VFS/issues/81) (closed, not planned) |
| [6. Sorting by dates and source](#line-6) | **Do next; most likely to be done soon.** A sort selector over the collected date (files now, bookmarks with VFS-6), unknown dates last, a source label. Two-date model later. | Sort selector «Relevancia / Más recientes / Más antiguos» in core and UI (acceptance criteria in the section) | Does «fecha» mean when it was published or when she saved it? | [#82](https://github.com/AndresHG/VFS/issues/82) |
| [7. Reorganising bookmarks](#line-7) | **Decided: will not do for now** (VFS never writes to a source). | — | — | [#83](https://github.com/AndresHG/VFS/issues/83) (closed, not planned) |
| [8. Bibliographic list](#line-8) | **Do later**, right after phase 3: offline extraction, XLSX + printable list. | ISBN/DOI and covers from EPUB/PDF, export to XLSX and HTML | An Excel sheet she keeps editing, a printed list, or both, and which Excel version? | [#84](https://github.com/AndresHG/VFS/issues/84) |

<a id="line-1"></a>

## Line 1. Thematic gathering: "reúne todo lo que tengo sobre X"

Issue: [#77](https://github.com/AndresHG/VFS/issues/77).

### 1. What she wants

She wants one request to gather everything she has on a topic, not files she can name:
- "Necesito estudiar todo lo que tengo sobre hígado graso" (ex. 1)
- "¿Qué recetas usan coco rallado junto con harina de almendra?" (ex. 2)
- "Reúne todo lo que tengo sobre propiedades y fuentes alimentarias del colágeno" (ex. 3)
- "Reúne lo que tengo sobre solmisación (el recolector debe entender el concepto)" (ex. 5)

**The gap.** Phase 1 matches words in file names and folder paths only. A PDF called
`Dra_Lopez_2019_dieta.pdf` that is all about esteatosis hepática, or a folder of recipes named
`postres sin gluten/`, will never match "hígado graso" or "coco rallado". Her requests have
three levels of difficulty:

| Level | Example | What is needed |
|---|---|---|
| A. Literal words in the text | ex. 2 (coco rallado AND harina de almendra), ex. 3 (colágeno) | Document full text (phase 3 / #24) |
| B. Same concept, different words | ex. 1: "esteatosis hepática", "EHGNA", "NAFLD", "MASLD", "fatty liver" | Synonyms / controlled vocabulary, or embeddings |
| C. Related concept, not a synonym | ex. 5: "solmisación" ↔ solfeo, do-re-mi, Guido d'Arezzo, hexacordo, "ut queant laxis" | Associative knowledge: embeddings partly, an LLM well |

Note that example 1 changed names twice: NAFLD (EHGNA in Spanish) was renamed MASLD in a 2023
multi-society Delphi consensus (236 experts, 56 countries) **[published, Rinella et al., J
Hepatol 2023, via PMC10856125]**. A 2015 PDF and a 2025 PDF on the same disease share almost no
vocabulary.

### 2. How it could work

**Option A: Spanish full text with BM25 (phase 3, #24). Required base for everything else.**
- SQLite FTS5 ships four tokenizers: `unicode61`, `ascii`, `porter`, `trigram`. The only stemmer,
  porter, "is designed for use with English language terms only" **[published, sqlite.org/fts5]**.
  There is **no built-in Spanish stemmer**.
- Spanish stemming options:
  1. `fts5-snowball` (BSD-3), a loadable extension wrapping libstemmer (`tokenize='snowball
     spanish'`) **[published, github.com/abiliojr/fts5-snowball]**. It is a C build per platform,
     loading from code is "untested" per its README, and recent maintenance is unclear (55 stars,
     22 commits).
  2. **Preferred:** stem in Rust with `rust-stemmers` (MIT/BSD-3, includes Spanish among 18
     languages; v1.2.0 from 2019, ~44 M downloads) **[published, crates.io and repo README]**.
     Register it as an FTS5 custom tokenizer through `fts5_tokenizer_v2`, or pre-stem the text
     into a shadow column and stem the query the same way. That needs no C extension and keeps
     one code path on Windows and macOS. Snowball Spanish is a light stemmer: it merges
     receta/recetas and graso/grasa, not hígado/hepático.
- BM25 is built in: `ORDER BY bm25(t, w1, w2…)` with per-column weights, so a title can outweigh
  the body (k1=1.2, b=0.75 are fixed) **[published, sqlite.org/fts5]**.
- **Example 2 is already solved by plain full text** if the recipes are text:
  `MATCH '"coco rallado" AND "harina de almendra"'`. Phrase plus AND is standard FTS5, and accent
  folding (`remove_diacritics 2`) handles "almendra"/"almendras" once stemmed. No AI is needed.
  But recipes that are photos, phone screenshots or scanned books contain no text until **OCR
  (#28)** runs. For a cooking archive this may be most of the recipes. The owner should check a
  sample.
- Example 3 (colágeno) is also mostly level A: "colágeno" is distinctive. BM25 plus a snippet
  answers it.

**Option B: offline concept expansion without a model (thesaurus).** At query time, expand
"hígado graso" into an OR over its synonyms, weighted below the literal terms.
- *Medical (ex. 1, 3):* **DeCS** (BIREME/PAHO; MeSH in Spanish, Portuguese, English, French):
  35,033 descriptors **[published, decs.bvsalud.org/en/about-decs]**. Edition 2025 has about
  735,000 terms including synonyms **[published, PAHO news 2025-04-30]**. The "Hígado Graso"
  record (id 5337, tree C06.552.241) lists the entry term "esteatosis hepática" and the English
  "Fatty Liver" / "Steatosis, Liver" **[published, decs.bvsalud.org/en/ths/resource/?id=5337]**.
  Its child descriptors (non-alcoholic fatty liver disease) supply EHGNA/NAFLD. Whether MASLD is
  in yet is **unverified**.
  **Licence:** not Creative Commons. It is a free licence agreement per edition, granted to the
  requesting institution and valid three years. It allows use and incorporation in an
  application. The data "shall not be sold or made available separately from the
  product/service/application", the PAHO/BIREME copyright must be credited, the edition stated,
  and the NLM MeSH terms accepted **[published, decs.bvsalud.org/?p=2123]**. That is workable for
  a private non-commercial app, but someone must request and sign the licence. Read the
  agreement before shipping.
- *General vocabulary (ex. 5):* the Spanish wordnet of the **Multilingual Central Repository**
  through Open Multilingual Wordnet: 38,512 synsets, 36,681 words, 57,764 senses, **CC BY 3.0**
  **[published, omwn.org/omw1.html]**. The MCR home page did not resolve when we tried it. A
  wordnet gives synonyms and hypernyms (solmisación → *notación musical*?), but it does **not**
  hold associative links like solmisación ↔ Guido d'Arezzo ↔ hexacordo. Whether "solmisación" is
  in it at all is **unverified**. Expect it to help ex. 1 and ex. 3 a little and ex. 5 hardly at
  all.
- *Personal synonym list:* a small editable file ("cuando busque X, busca también Y"), like
  Recoll's synonym support. It is cheap and transparent but needs the owner to curate it.

**Option C: local embeddings (semantic search, #41).** Encode text chunks and the query into
vectors and rank by cosine similarity, fused with BM25 (hybrid, e.g. reciprocal rank fusion).

| Model | Params | Dim | Max tokens | Licence | Notes |
|---|---|---|---|---|---|
| multilingual-e5-small | ~118 M (12 layers, hidden 384, vocab 250k) | 384 | 512 | MIT | Needs "query:"/"passage:" prefixes; MIRACL-es nDCG@10 0.817 |
| paraphrase-multilingual-MiniLM-L12-v2 | ~118 M | 384 | 128 | Apache-2.0 | Short context: weak for document chunks |
| bge-m3 | ~568 M (XLM-R large) | 1024 | 8192 | MIT | Dense + sparse + multi-vector; ONNX ~2.2 GB fp32 |

Sources: [published] HF config and cards for e5-small (config.json: 12 layers / 384 / 250,037
vocab; licence MIT), MiniLM card (384-d, 128 tokens, Apache-2.0), bge-m3 card (1024-d, 8192
tokens, MIT, ONNX files present), MIRACL figure from the elastic/multilingual-e5-small-optimized
card. The 118 M and 568 M counts come from HF metadata and the BGE-M3 paper. The 2.2 GB is an
**[estimate]**: 568 M × 4 bytes.
- Runtime: `fastembed-rs` (Apache-2.0) runs all three through `ort` (ONNX Runtime) and needs no
  async runtime. It has quantized variants for some models (`BGEM3Q` is dynamically quantized and
  batch-dependent) **[published, github.com/Anush008/fastembed-rs]**. Storage: `sqlite-vec`
  (Apache-2.0/MIT, Windows supported, float/int8/binary vectors, `cargo add sqlite-vec`) is
  **pre-v1, "expect breaking changes"** **[published, github.com/asg017/sqlite-vec]**.
- What it buys: level B well ("hígado graso" lands near "esteatosis hepática"), level C partly.
  Small models know "solfeo" is near "solmisación", but whether they tie in "hexacordo" or Guido
  is doubtful. It also brings false positives the owner cannot explain.

**Option D: LLM query expansion with her own API key (phase 5, #38/#40).** Send only the query
("solmisación") to the model. It returns a list of Spanish/English/Latin terms and names: solfeo,
do re mi, ut re mi fa sol la, Guido d'Arezzo, hexacordo, mano guidoniana, "Ut queant laxis".
VFS then runs those terms as a weighted OR through local BM25. **No document leaves the PC**,
and the expansion is shown to her and editable ("También busco: …"). This is the only option
that handles level C well and stays cheap. It fails without internet or a key.

**Option E: full RAG.** Retrieve chunks locally, send them to the model, get a synthesis ("what
do my documents say about hígado graso"). That is #39/#40. Document text leaves the PC, so
consent is needed per send. It is useful for "necesito estudiar", but it is a different product
from gathering.

**The product concept: saved collections ("dossiers").** All of her requests end in "reúne":
she wants a durable pile, not a one-off result list. Proposal: a *Colección* is a named, saved
query (terms plus expansion plus filters plus sources), plus results she pinned or excluded by
hand, re-run on demand so new files appear. It can be exported, as new files in a place she chooses (never inside or onto a source), as a folder of shortcuts
(`.lnk` on Windows, aliases or symlinks on macOS, no copies) or as a printable list. Prior art:
DEVONthink smart groups ("itself just a saved search", local or global) **[published,
devontechnologies.com blog]**; Zotero collections (manual) plus saved searches; Recoll
(query-time stemming, Windows installer) **[published, recoll.org/pages/features.html]**. The
pinned/excluded overlay is what makes it trustworthy: the machine proposes, she disposes.

### 3. What it needs from the user

- A, B, C: nothing beyond indexing time and disk. For DeCS, the developer signs the BIREME
  licence agreement.
- D, E: her own Anthropic or OpenAI key (phase 5 flow) and consent. For D the consent covers
  only the query text. For E it covers document excerpts.
- Collections: a name per collection. Optionally she confirms or edits the suggested expansion
  terms.

### 4. Privacy and terms-of-service constraints

- A, B and C are fully offline. Embedding model weights are MIT/Apache, and the models can ship
  in the installer or download once at opt-in.
- DeCS: no separate redistribution of the data, credit PAHO/BIREME, state the edition, no
  endorsement implied, licence renewal per edition (3 years). MCR/OMW: CC BY 3.0, attribution in
  "Acerca de".
- D sends a short topic string. Even that can be sensitive (a health topic such as "hígado
  graso"), so show it before sending, per the phase 5 rule.
- E sends personal document text and needs per-send consent. Check the provider's data-retention
  terms in #38.

### 5. Cost on the target PC and API fees

Assumptions **[estimate]**: 300,000 text-bearing documents out of 5 M files, extracted text
averaging 8 chunks of ~300 tokens, so 2.4 M chunks. Older 4-core CPU, maybe no AVX2.
- **A, full text:** covered by phase 3's budget. Stemming in Rust adds negligible CPU against
  extraction I/O on the HDD.
- **B, thesaurus:** DeCS Spanish subset plus OMW Spanish as a small SQLite lookup table, about
  10–40 MB **[estimate: ~735k DeCS terms × ~40 bytes, plus ~58k wordnet senses]**. Query cost:
  microseconds.
- **C, embeddings:** Elastic reports ~0.03–0.07 (units unstated, presumably seconds) per input of
  50–400 characters for e5-small on an unnamed Intel CPU **[published,
  elastic/multilingual-e5-small-optimized]**. A 300-token chunk is ~1,200 characters, so assume
  ~0.2 s per chunk per thread on the old CPU. With 3 threads that is ~15 chunks/s, so
  2.4 M chunks take about 45 h of full CPU **[estimate]**, spread over weeks at low priority
  beside Synology. Storage at 384 dimensions: float32 ≈ 3.7 GB, int8 ≈ 0.9 GB, binary ≈ 115 MB
  **[estimate: 2.4 M × 1536 / 384 / 48 bytes]**. Brute-force int8 search over 2.4 M vectors
  scans ~0.9 GB per query: too slow and too RAM-heavy on 8 GB with an HDD unless binary
  pre-filter plus rescoring is used. bge-m3 needs ~15× the transformer compute of e5-small
  **[estimate: ~300 M vs ~21 M non-embedding params]**, so weeks of CPU: **not viable** on the
  target. The model file adds ~120 MB (int8) to ~470 MB (fp32) for e5-small **[estimate:
  118 M × 1 or 4 bytes]**. Whether ONNX Runtime runs well without AVX2 needs a check on the
  target CPU.
- **D, query expansion:** about 600 input tokens (instructions plus query) and 300 output tokens
  (term list). Claude Haiku 5.5 at $0.10 / $0.50 per MTok **[published,
  platform.claude.com/docs/en/about-claude/pricing]** comes to **≈ $0.0002 per query**, so
  $0.21 per 1,000 queries **[estimate]**. Claude Sonnet 5.5 at $2 / $10 comes to ≈ $0.004 per
  query. OpenAI gpt-6-luna at $0.10 / $0.50 **[published, developers.openai.com/api/docs/pricing]**
  costs the same as Haiku 5.5. Anthropic notes newer models' tokenizer yields ~30% more tokens,
  which is still a fraction of a cent.
- **E, RAG:** 20 chunks × 500 tokens = 10k input + 1k output. Haiku 5.5 ≈ $0.0015 and Sonnet 5.5
  ≈ $0.03 per question **[estimate]**.

### 6. Dependencies in VFS

Phase 3 text extraction and #24 Spanish BM25 are the hard prerequisites. #28 OCR is needed for
image or scanned recipes and articles. #41 embeddings evaluation equals option C. #38, #40 and
#39 are options D and E. The source selector from VFS-6 lets collections span files and
bookmarks. Bookmark titles, URLs and possibly page text join the same FTS index.

### 7. Smallest useful first step

Inside phase 3 / #24: Spanish stemming via `rust-stemmers` in an FTS5 tokenizer, AND/phrase
queries, and a golden test set built from her six sentences against a sample of her real files.
That measures how far level A gets before any concept work. Then, as a separate S/M ticket,
"Colecciones": save a query plus pins and exclusions, and export it as a printable list or a shortcut folder written where she chooses. It works
with name search today and grows with full text.

### 8. Recommendation

**Do next:** full text with Spanish stemming (already planned) plus saved collections, which
are cheap and serve every example. **Do later:** LLM query expansion (D) as the first phase 5
feature. It is the best value for level C, costs fractions of a cent, and sends no documents.
After that a DeCS-based medical expansion if the licence is acceptable. **Do not do now:** local
embeddings on the target PC. Days of CPU, a GB-scale index, a pre-v1 vector store and modest
gain over BM25 plus expansion. Revisit in #41 with a measured benchmark on her machine.

### 9. Open product questions for the owner

1. **Is it acceptable that "understanding the concept" (ex. 5) uses the internet: only the
   search words, never her documents, sent with her API key?** Or must it be strictly offline,
   accepting weaker results?
2. What share of her recipes and articles are images or scans (decides #28's priority)?
3. Should a collection be live (re-run, new files appear) or frozen once saved?
4. Export format: a folder of shortcuts, a printable list, or both?

### 10. Sources

- https://www.sqlite.org/fts5.html
- https://github.com/abiliojr/fts5-snowball
- https://crates.io/crates/rust-stemmers ; https://github.com/CurrySoftware/rust-stemmers
- https://decs.bvsalud.org/en/about-decs/ ; https://decs.bvsalud.org/?p=2123 ;
  https://decs.bvsalud.org/en/ths/resource/?id=5337
- https://www.paho.org/es/noticias/30-4-2025-decs-edicion-2025-esta-disponible
- https://omwn.org/omw1.html ; https://github.com/omwn/omw-data/blob/main/wns/mcr/LICENSE
- https://www.ncbi.nlm.nih.gov/pmc/articles/PMC10856125/ (NAFLD → MASLD)
- https://huggingface.co/intfloat/multilingual-e5-small (and /raw/main/config.json)
- https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2
- https://huggingface.co/BAAI/bge-m3 ; https://arxiv.org/abs/2402.03216
- https://huggingface.co/elastic/multilingual-e5-small-optimized
- https://github.com/Anush008/fastembed-rs ; https://github.com/asg017/sqlite-vec
- https://platform.claude.com/docs/en/about-claude/pricing
- https://developers.openai.com/api/docs/pricing
- https://devontechnologies.com/blog/searchglobalsmartgroups ;
  https://www.recoll.org/pages/features.html

<a id="line-2"></a>

## Line 2. Finding unnamed photos by what they show, or where and when they were taken

Issue: [#78](https://github.com/AndresHG/VFS/issues/78).

> **Decided (owner, 2026-11-20): out of scope.** Image, audio and video content stay out of scope; photos are found by name and folder as today. This section is kept as a future line: what finding unnamed photos would take and what it would cost, so the question can be revisited with evidence if the owner ever chooses to. It is not a recommendation to reopen the decision.

### 1. What she wants

- Example 4: «Reúne todas las imágenes que tengo de la cocina de Valle de Ansó (sin nombre, en carpetas por año o sueltas)».
- Example 6: «Todas las fotografías de localizar y quitar la fosa séptica antigua e instalar la nueva, por orden cronológico».

Neither photo set has a useful file name. Both requests combine three things: a **place** (the house in Ansó, or home), a **time span** (a building job lasting some weeks) and **content** (a kitchen; digging, pipes, a tank). Search by name and path (Phase 1) and document text (Phase 3) cannot reach any of the three.

### 2. How it could work

> Kept for reference: line 2 is out of scope by the owner's decision.

#### 2a. Free metadata: EXIF date, GPS, camera (no model)

- **Fields.** Phone and camera JPEG/HEIC files carry `DateTimeOriginal` (0x9003), `OffsetTimeOriginal` (0x9011, the time zone), `SubSecTimeOriginal`, `Model` (0x0110), a GPS IFD with latitude and longitude, and often an embedded IFD1 thumbnail (`ThumbnailOffset`/`ThumbnailLength`) ([ExifTool EXIF tags](https://exiftool.org/TagNames/EXIF.html)).
- **Cost to read.** In a JPEG, EXIF sits in the APP1 segment near the start of the file. A JPEG marker length field is 16 bits, so APP1 is at most about 64 KB. One small read per file is enough. HEIC keeps EXIF as an item in the `meta` box, which is usually near the start; that position is an assumption to verify on her files.
- **Rust crates.** `kamadak-exif` (BSD-2-Clause) reads TIFF, JPEG, HEIF, PNG and WebP ([docs.rs](https://docs.rs/kamadak-exif/latest/exif/)). `nom-exif` (MIT) also covers HEIC/AVIF and MP4/MOV video, offers a lazy iterator with early exit after the GPS tags, and accepts any `Read + Seek` ([README](https://docs.rs/crate/nom-exif/latest/source/README.md)). Neither documents how many bytes it reads. VFS should pass a capped 64–128 KB buffer itself rather than open the whole file.
- **Coverage is uneven:**
  - Photos taken by phone: date almost always, GPS when the camera had location on.
  - Scans of paper prints: no capture date for the scene.
  - Older digital cameras: date but no GPS.
  - WhatsApp in normal photo mode: reportedly removes GPS and most EXIF; "send as document" keeps it. Only secondary blog sources say this ([fast.io](https://www.fast.io/resources/whatsapp-telegram-signal-photo-metadata-preservation/), [wildandfreetools](https://wildandfreetools.com/blog/does-whatsapp-remove-exif-data)). We found no Meta primary source, so it is **unverified**.
  - We found **no published statistic** on what share of a family archive has GPS. Step 7 measures it on her archive.
- **Date fallback chain:**
  1. `DateTimeOriginal` (+ `OffsetTimeOriginal`).
  2. A date in the file name: `IMG_20190612_143005.jpg` (Android), `IMG-20190612-WA0003.jpg` (WhatsApp, the date it was received), `PXL_…`, `Screenshot_…`.
  3. A year in the folder name (`2019`, `Fotos 2019`).
  4. `mtime`, already in the Phase 1 index and flagged as least reliable.

  The UI should show which source each date came from.

**HDD cost [estimate]**

- Assumptions: 300,000 photos; one random 64 KB read each; 5,400 rpm disk ≈ 11 ms seek + 5.6 ms rotational latency ≈ 17 ms; 7,200 rpm ≈ 13 ms; plus NTFS metadata lookups and contention from the Synology client.
- Result: 300,000 × 15–30 ms ≈ **1.3–2.5 hours, once**, as a throttled background job that reads files in directory order. After that, only new files, at seconds per day.

**Offline reverse geocoding**

- **Data.** GeoNames `cities500.zip` (**13.9 MB** zip, **41.8 MB** text, **237,329** places [measured, downloaded 2026-11-20]) or `cities1000` (11 MB); both CC BY 4.0 ([dump index](https://download.geonames.org/export/dump/)). Places in `cities500` have more than 500 inhabitants or are administrative seats down to PPLA4. A k-d tree over ECEF coordinates gives the nearest place, as the [reverse_geocoder](https://github.com/thampiman/reverse-geocoder) project does (that project is LGPL, so reimplement it; the method is small).
- **Ansó resolves [measured].** `cities500` contains *Ansó* (PPLA3, 42.7579 N, −0.8295 W). The Spain file `ES.txt` (3.2 MB zip) also contains the valley as a feature: *Valle de Ansó* (T.VAL, 42.7625, −0.8304).
- **Precision limit [measured].** Neighbouring seats are close: Fago is 5.0 km from Ansó and Echo 6.8 km. A photo taken up the valley toward Zuriza can resolve to the wrong village. A nearest-place name is good for display ("Ansó, Huesca") but weak as a filter. The better filter is **«cerca de aquí»**: a radius (e.g. 300 m to 2 km) around a point she picks, by choosing one known photo («más fotos tomadas aquí») or a place name.
- **Her examples:**
  - Example 4 works on GPS alone **if** the kitchen photos were taken by phone at the house. VFS can return every photo within 300 m of the house as a thumbnail grid for her to scan. Scanned prints and old cameras will not appear.
  - Example 6 works on **GPS + time clustering** with no content understanding: photos within ~200 m of home, inside the weeks of the job, in `DateTimeOriginal` order. The weak point: family and garden photos taken at home in the same weeks get mixed in. That is where content ranking (2b) helps.

**Event grouping**

- Start a new event when the time gap exceeds a threshold (e.g. 4–8 h) or the location jumps (e.g. more than 1 km). This is the same idea as the "moments" in Apple and Google Photos.
- Show an event as a group card: date range, place name, photo count. This also gives «fotos del mismo día/obra que esta».

#### 2b. Content search with image–text embeddings (CLIP family), run locally

The model maps an image and a Spanish sentence («una cocina», «excavación de una fosa séptica») into one vector space. Search is a nearest-neighbour lookup.

| Model | Licence | Spanish | Image cost | Note |
|---|---|---|---|---|
| OpenAI/OpenCLIP ViT-B/32 | MIT | No (English only) | **8.8 GFLOPs** **[published]** at 224 px; 88 M image + 63 M text params ([open_clip profile](https://raw.githubusercontent.com/mlfoundations/open_clip/main/docs/model_profile.csv)) | Figures of 4.4 seen elsewhere are GMACs (= 8.8 GFLOPs). Immich's default. |
| XLM-RoBERTa-base + ViT-B/32 (OpenCLIP, LAION) | MIT | Yes; Spanish recall **75.99 %** [published, Immich] | Same 8.8 GFLOPs image side; text encoder 278 M params | Cheapest multilingual option. |
| SigLIP 2 ViT-B/16 (multilingual WebLI) | Apache-2.0 ([HF](https://huggingface.co/google/siglip2-base-patch16-224)) | Spanish recall **83.61 %** [published, Immich] | ≈35 GFLOPs (ViT-B/16 class) **[estimate]** | Best quality per cost; ~4× the compute of B/32. |
| nllb-clip-base-siglip | (not verified) | 80.91 % [published, Immich] | Image 8.8 GFLOPs; text 414 M params | Heavy text side. |
| M-CLIP XLM-R-Large + ViT-B/32 | Not stated on the card | Spanish COCO R@10 89.1 [published] ([HF](https://huggingface.co/M-CLIP/XLM-Roberta-Large-Vit-B-32)) | Image 8.8 GFLOPs | Licence unclear, so avoid. |
| MobileCLIP / MobileCLIP2 S0–S2 (Apple) | Code MIT; **weights under Apple ML Research Model Terms** ([repo](https://github.com/apple/ml-mobileclip)) | English only | S0: 11.4 M image params, 1.5 ms on the repo's (unnamed) device **[published]**; IN-1k 67.8–77.2 % | Fastest. Weight licence is likely research-only, so unsuitable unless verified. |
| Jina CLIP v2 | **CC BY-NC 4.0** | 89 languages | 0.9 B params, 512 px ([HF](https://huggingface.co/jinaai/jina-clip-v2)) | Too heavy for the target PC. |

- **Spanish recall** is from Immich's multilingual benchmark on a Ryzen 7800X3D ([Immich searching](https://docs.immich.app/features/searching)). Immich lists ~3 GB of RAM **[published]** for the B-size multilingual models in that table. On an 8 GB PC that is a real cost.
- **Quality, honestly.** "Kitchen" is an easy CLIP concept. "Septic tank work" is not a standard benchmark class. It will probably rank as "excavator / trench / pipes / concrete tank", which may be good enough as a ranking signal **inside** a GPS + date filter. That has to be tested on ~20 of her real photos before anything is promised.

**Published CPU benchmark**

- Immich with ViT-B/32 on an Intel i7-8559U (4 cores / 8 threads, 2018, no GPU) indexed 51,126 pre-made previews in **57 min 39 s ≈ 14.8 images/s**. The median model call was **0.124 s**; one text query took **0.038 s** [published, [mustafa.net](https://mustafa.net/2026/10/10/immich-without-gpu-intel-nuc/)].
- That blog is a single source, and its figures exclude generating the previews.

**Target PC [estimate]**

- **Model time.** Assume an older 4-core with maybe no AVX2, at 3–4× slower than the i7-8559U, so ~4 img/s for B/32. 300,000 photos ≈ **21 h of full CPU**. SigLIP2-B/16 at ~4× the FLOPs ≈ **3–4 days**.
- **Decoding.** A 12 MP JPEG must be decoded first. Full decode is roughly 60–150 ms on an old CPU **[estimate]**. libjpeg-turbo's IDCT scaling decodes straight to 1/8 size (4000×3000 → 500×375); 1/2, 1/4 and 1/8 are the fast cases ([libjpeg-turbo README](https://chromium.googlesource.com/chromium/deps/libjpeg_turbo/+show/refs/heads/main/README.md)). No published timing; we assume ~10–20 ms **[estimate]**.
  - The embedded EXIF thumbnail (often only 160×120) avoids reading the whole file. It is below CLIP's 224 px, so quality must be measured.
  - HEIC needs an HEVC decoder. Codec licensing on Windows is an open issue.
- **I/O.** Reading 300,000 originals at ~4 MB is ~1.2 TB: ~4–6 h on a busy HDD.
- **Total:** realistically **1–2 days of throttled background work** for B/32, spread over idle time across a week or more. New photos later cost seconds each, which the target PC can easily do locally.
- **Storage:** 512-d float32 = 2 KB per photo = **614 MB** for 300,000; int8 = 154 MB; binary = 19 MB. `sqlite-vec` stores these in SQLite. It is brute-force only and supports binary quantisation with oversample-and-rescore ([sqlite-vec BQ guide](https://alexgarcia.xyz/sqlite-vec/guides/binary-quant.html)). A brute-force scan of 300,000 × 512 int8 is ~0.1–0.3 s [estimate].

#### 2c. Compute the embeddings on the owner's Mac, ship the vectors

- **How it works.** Her files are already synced to the Synology. The owner's Apple Silicon Mac can read them from the NAS over the LAN, not from the slow PC, and run the same model's image encoder overnight.
- **Time [estimate].** I/O bound: ~1.2 TB over gigabit ≈ 3–4 h; Wi-Fi is slower. Model time on Apple Silicon is a small share; we found no published figure.
- **What ships to the PC:** an int8 vector file (~150 MB) keyed by path + size + mtime, plus only the **text** encoder in ONNX Runtime CPU. That is ~65 MB int8 for B/32's text encoder, or ~280 MB for XLM-R-base [estimate]. A query costs well under a second.
- **New photos:** embedded locally on the PC, a few per day.
- **Constraint:** both machines must use the identical model and preprocessing.

#### 2d. Captions through an API (owner's key, Phase 5)

- **How it works.** Send each photo downscaled to ~512 px. Ask for a short Spanish caption plus keywords («cocina, azulejos, fogón de leña»). Store the text and index it with Phase 3's Spanish BM25. **Queries then stay offline.** Captions are also readable, so she can see *why* a photo matched.
- **Why not API embeddings.** Gemini Embedding 2 costs only $0.00006 per image in batch **[published]** ([pricing](https://aistudio.google.com/docs/pricing)), but every query would then also need the API online. That breaks VFS's offline rule.

### 3. What it needs from the user

> Kept for reference: line 2 is out of scope by the owner's decision.

- **A (metadata):** nothing. Optionally she picks "home" once by choosing a photo taken there.
- **B:** acceptance of a long background job and ~0.2–0.6 GB of extra disk; a pause/resume control.
- **C:** the owner runs a Mac job on LAN access to the NAS, then copies one file across.
- **D:** an API key, explicit consent per folder set, and a preview of what leaves the PC (Phase 5 rule).

### 4. Privacy and terms-of-service constraints

> Kept for reference: line 2 is out of scope by the owner's decision.

- **A, B and C** keep everything local: no data leaves the PC, and C leaves only the owner's own LAN.
- **D sends family photos off the PC.** They may include children, house interiors and photographed ID documents.
  - Anthropic: API data deleted within 30 days and not used for training by default ([retention](https://privacy.claude.com/en/articles/7996866-how-long-do-you-store-my-organization-s-data), [training](https://privacy.claude.com/en/articles/7996868-is-my-data-used-for-model-training)).
  - OpenAI: abuse logs kept up to 30 days; Batch inputs and files kept until deleted; Batch is not eligible for zero data retention; no training by default ([your data](https://developers.openai.com/api/docs/guides/your-data)).
  - Google: the paid tier does not use prompts to improve products and logs them "for a limited period". The **free tier may be used for training and human review** ([Gemini API terms](https://aistudio.google.com/docs/terms)). The free tier must never be used for this.
- **Face recognition: not recommended.** Biometric data used to identify a person is a special category under GDPR Art. 9(1) ([text](https://gdpr-info.eu/art-9-gdpr/)). It concerns relatives and third parties who have not consented, and adds model weight and liability for a need she has not stated.

### 5. Cost (300,000 photos)

> Kept for reference: line 2 is out of scope by the owner's decision.

**Local options [estimate]:**
- A: 1.3–2.5 h of disk time once, plus ~15–30 MB for the geodata and index columns.
- B: 1–2 days of throttled CPU (B/32) or several days (SigLIP2-B/16); 0.15–0.6 GB of vectors; ~0.3–1 GB of RAM while the job runs.
- C: one Mac night; ~150 MB to ship.

**API option, Batch API [estimate].** Assumptions: images 512×384 (Gemini 384×288); a 150-token prompt (no caching assumed); 60 output tokens; gpt-5-mini with minimal reasoning (reasoning tokens would add cost).

| Provider / model | Image tokens (from the docs' formulas) | Batch price, input / output per MTok | Total |
|---|---|---|---|
| Claude Haiku 5.5 | ⌈512/28⌉·⌈384/28⌉ = 19·14 = 266 | $0.05 / $0.25 | 124.8 M in → $6.24; 18 M out → $4.50; **≈ $11** |
| Claude Haiku 4.5 | 266 | $0.50 / $2.50 | **≈ $107** |
| Gemini 2.5 Flash-Lite | 258 (≤384 px) | $0.05 / $0.20 | **≈ $10** |
| gpt-5-mini | 16·12 patches × 1.2 = 231 | $0.125 / $1.00 | **≈ $32** |
| gpt-5-nano (marked deprecated) | 192 × 1.5 = 288 | $0.025 / $0.20 | **≈ $7** |

- **Formula and price sources.** Claude: ⌈w/28⌉×⌈h/28⌉ visual tokens ([vision](https://platform.claude.com/docs/en/build-with-claude/vision)). This replaces the older w·h/750 rule. Haiku 5.5 is $0.10/$0.50 standard and $0.05/$0.25 batch ([pricing](https://platform.claude.com/docs/en/about-claude/pricing)). OpenAI patch formula and multipliers: [images-vision](https://developers.openai.com/api/docs/guides/images-vision). Avoid gpt-4o-mini: it costs 2,833 tokens per image even at low detail. OpenAI prices: [pricing](https://developers.openai.com/api/docs/pricing). Gemini: 258 tokens per image ≤384 px ([image understanding](https://aistudio.google.com/docs/image-understanding)).
- **Upload [estimate]:** ~40 KB per photo ≈ 12 GB, about 3 h at 10 Mbit/s upstream.
- **Cheaper hybrid:** caption only a metadata-filtered candidate set at query time, e.g. 2,000 photos near home in 2021, for **cents**. Far less data leaves the PC.

### 6. Dependencies in VFS

> Kept for reference: line 2 is out of scope by the owner's decision.

- **Phase 1:** path, size and mtime as cache keys; the new file-name date patterns live here.
- **Phase 3:** the isolated worker process, needed because image and EXIF parsers run on untrusted files; Spanish BM25 for captions. Taking this line up would reverse the "no image content" exclusion, which the owner has kept.
- **Phase 5 / #38:** the consent screen and key storage for D.
- **#41 and #28:** this line is the concrete evaluation those issues ask for.
- **VFS-6 source selector:** add «Fotos».
- **New UI:** a thumbnail grid and an event/group view; current results are lists. Generate thumbnails lazily, only for displayed results.

### 7. Smallest useful first step

Only if the owner ever revisits the decision: a **metadata-only photo index**, with no model:
- one capped EXIF read per JPEG/HEIC in the worker;
- columns for capture date (with its source), GPS, camera and orientation;
- the GeoNames `cities500` place name;
- a «cerca de esta foto» radius filter, a date-range filter, chronological sort and event grouping;
- a thumbnail grid for results.

It ships with a **coverage report** for her archive: the share of photos with a capture date and with GPS. That number decides whether B/C is worth doing. Cost: one EXIF read per photo, about 2 h once.

### 8. Recommendation

**Not scheduled: out of scope by the owner's decision.** If it is ever revisited, the evidence points to this order:

- **First: option A.** It is cheap, offline, has no model, and may already answer examples 4 and 6 for phone photos.
- **Then, only with evidence: option C** (Mac-computed SigLIP2 or XLM-R B/32 vectors, local text encoder), only if A's coverage report and a 20-photo test show that content ranking adds recall. B is the fallback if the Mac route is impractical.
- **D (API captions):** only as the opt-in, query-time hybrid on a filtered subset, never a bulk upload by default.
- **Do not do:** face recognition.

| Option | One-time cost | Quality for ex. 4 / ex. 6 | Privacy |
|---|---|---|---|
| A metadata only | ~2 h disk; ~30 MB | Good when phone GPS exists; nothing for scans or WhatsApp copies; no content | Fully local |
| B + local CLIP on target PC | 1–4 days of throttled CPU; 0.2–0.6 GB; RAM pressure | Adds "kitchen" for photos without GPS; niche concepts uncertain | Fully local |
| C + CLIP computed on the Mac | One night on the Mac + LAN; ~150 MB shipped | Same as B; can afford the better SigLIP2 model | Local to the owner's LAN |
| D API captions | ≈ $7–32 batch for all 300,000 (cents for a subset); ~12 GB upload | Best on niche concepts; readable captions; offline queries afterwards | Photos leave the PC (30-day retention typical) |

### 9. Open product questions for the owner

None now: the owner has decided. If the line is ever revisited, these are the questions:

1. Would metadata alone (A: dates, GPS, place names, no pixels read) be acceptable, given that her examples 4 and 6 cannot be answered by name?
2. Is the Mac + NAS route (C) acceptable to run, and to re-run after a model change?
3. Would she accept any photos leaving the PC (D), and if so, only a filtered subset after a preview?
4. Roughly how many photos are there, and what share is phone photos versus scans? (Step 7 will measure it.)

### 10. Sources

- https://exiftool.org/TagNames/EXIF.html
- https://docs.rs/kamadak-exif/latest/exif/
- https://docs.rs/crate/nom-exif/latest/source/README.md
- https://www.fast.io/resources/whatsapp-telegram-signal-photo-metadata-preservation/ (secondary)
- https://wildandfreetools.com/blog/does-whatsapp-remove-exif-data (secondary)
- https://download.geonames.org/export/dump/ (and readme.txt, cities500.zip, ES.zip; measured 2026-11-20)
- https://github.com/thampiman/reverse-geocoder
- https://raw.githubusercontent.com/mlfoundations/open_clip/main/docs/model_profile.csv
- https://github.com/apple/ml-mobileclip
- https://huggingface.co/google/siglip2-base-patch16-224
- https://huggingface.co/jinaai/jina-clip-v2
- https://huggingface.co/M-CLIP/XLM-Roberta-Large-Vit-B-32
- https://docs.immich.app/features/searching
- https://docs.immich.app/features/reverse-geocoding
- https://mustafa.net/2026/10/10/immich-without-gpu-intel-nuc/
- https://chromium.googlesource.com/chromium/deps/libjpeg_turbo/+show/refs/heads/main/README.md
- https://alexgarcia.xyz/sqlite-vec/guides/binary-quant.html
- https://platform.claude.com/docs/en/build-with-claude/vision
- https://platform.claude.com/docs/en/about-claude/pricing
- https://developers.openai.com/api/docs/guides/images-vision
- https://developers.openai.com/api/docs/pricing
- https://developers.openai.com/api/docs/guides/your-data
- https://aistudio.google.com/docs/pricing
- https://aistudio.google.com/docs/image-understanding
- https://aistudio.google.com/docs/terms
- https://privacy.claude.com/en/articles/7996866-how-long-do-you-store-my-organization-s-data
- https://privacy.claude.com/en/articles/7996868-is-my-data-used-for-model-training
- https://gdpr-info.eu/art-9-gdpr/
- https://support.microsoft.com/en-us/Windows/Apps/Photos/search-photos (Windows Photos: location search; semantic search only on Copilot+ PCs)
- https://support.apple.com/guide/photos/search-for-photos-pht7b4e05224/mac

<a id="line-3"></a>

## Line 3. Her private YouTube playlists

Issue: [#79](https://github.com/AndresHG/VFS/issues/79).

### 1. What she wants

She wants one search over several sources, including "sus listas de reproducción privadas de YouTube". The search should separate what she already has from what she does not, and sort by publication date, collection date and source. With her example "Necesito estudiar todo lo que tengo sobre hígado graso", a video saved months ago in a private playlist such as "Salud" should appear next to her PDFs and bookmarks. It should show its title, channel, publication date and the date she added it, plus a link that opens it.

### 2. How it could work

**Option A: Google Takeout import, no API and no Google Cloud project.**
- She requests "YouTube y YouTube Music → listas de reproducción" (optionally history and subscriptions) at takeout.google.com, downloads the zip, and points VFS at it (or at a folder VFS already indexes).
- Format **[published]**, Google Data Portability schema (the same data as Takeout, https://developers.google.com/data-portability/schema-reference/youtube): playlists are CSV. Per-playlist video rows carry only `Video ID` and `Playlist Video Creation Timestamp`, the date she added it. A separate playlists CSV carries Playlist ID, title, description, visibility and create/update timestamps. A community report says Takeout produces n+1 files for n playlists, and that the header language follows the Takeout UI language (https://github.com/iv-org/invidious/issues/4069, FreeTube issue #5057). **The playlist rows have no video titles** as far as the schema shows. Google's schema page mislabels some sections, so this needs confirming on a real export.
- Watch history is exported as JSON or HTML (`watch-history.json` with `title`, `titleUrl`, `time`, `subtitles[]` = channel) **[published]** per the My Activity schema (https://developers.google.com/data-portability/schema-reference/my_activity). Unlike playlists, history *does* carry titles.
- Titles and thumbnails for playlist rows would come from YouTube's oEmbed endpoint (`https://www.youtube.com/oembed?url=…&format=json`, no key). One test call **[measured, 2026-11-20, one public video]** returned `title`, `author_name`, `author_url`, `thumbnail_url`. It does **not** return the publication date, and it only works for public or unlisted videos.
- An alternative is `videos.list` with a plain API key (1 unit per 50 IDs **[published]**), which returns `snippet.title`, `snippet.publishedAt` and thumbnails. That needs a Cloud project and an API key, and it brings in the 30-day storage rule (section 4).
- Trade-off: it is a one-off snapshot, and she must repeat Takeout to refresh. It needs no Google consent screen and no tokens, and it is the only route that also gives **watch history**.

**Option B: YouTube Data API v3 with OAuth (live sync).**
- `playlists.list?mine=true&part=snippet,contentDetails` and then `playlistItems.list?playlistId=…&part=snippet,contentDetails` (maxResults 50, 1 unit per call) **[published]** (https://developers.google.com/youtube/v3/docs/playlists/list, https://developers.google.com/youtube/v3/determine_quota_cost).
- `playlistItems` gives title, description, the video owner's channel, thumbnails, `snippet.publishedAt` (added-to-playlist date) and `contentDetails.videoPublishedAt`. That covers both of her sort dates.
- `mine` requires an authorized request, and private playlists cannot be read with an API key.
- **Watch Later and History are not available.** Since 2016-09-12 the API returns the placeholder IDs `HL`/`WL`, and list calls on them return empty lists (https://developers.google.com/youtube/v3/revision_history, entry of 2016-09-15). `playlists.list` also documents `playlistOperationUnsupported` with "you can't list your watch later playlist". If she keeps things in "Ver más tarde", only Takeout can see them (Takeout's coverage of Watch Later is also unverified).
- Transcripts: the Data API offers no transcripts for other people's videos. `captions.download` works only for captions the caller can edit, which in practice means her own uploads. Out of scope.
- OAuth for a desktop app **[published]** (https://developers.google.com/identity/protocols/oauth2/native-app): loopback redirect `http://127.0.0.1:<random port>`, PKCE with S256 ("Always use PKCE"). Desktop apps "are public clients and cannot keep secrets confidential", so the bundled client secret is not a secret. Store the refresh token in Windows Credential Manager / macOS Keychain via the `keyring` crate, as in Phase 5.
- Trade-off: it gives live refresh and full metadata, but it adds a Google Cloud project, a consent screen, verification questions and the 30-day storage rule.

**Option C: Google Data Portability API.** It covers the same content as Takeout but programmatically. It requires app verification, annual re-verification, a security assessment for restricted scopes and billing enabled **[published]** (https://developers.google.com/data-portability/user-guide/overview). That is far too heavy for one user. **Do not do.**

### 3. What it needs from the user

- Option A: a Google login in her browser, the Takeout request form (a few clicks), waiting for the email, downloading the zip and choosing it in VFS. This is a one-time, guided flow that someone could do for her.
- Option B: someone (the owner) creates a Google Cloud project. Ideally it is created **while signed in to her own Google account** with user type **Personal** (see section 4), the YouTube Data API is enabled, and a Desktop OAuth client is created. She then clicks "Conectar YouTube" once and approves `youtube.readonly` in her browser.

### 4. Privacy and terms-of-service constraints

- **Scope class:** `youtube.readonly` ("View your YouTube account"). We could not find Google's own table saying whether it is "sensitive" or "restricted". Google's docs list YouTube scopes as needing review, and secondary sources call all YouTube scopes sensitive (https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification). **Unverified; treat it as sensitive.** Check the Cloud Console Data Access page.
- **Testing status** **[published]** (https://support.google.com/cloud/answer/15549945): up to 100 test users, and authorizations *and refresh tokens expire 7 days after consent* unless only name/email/profile scopes are requested. She would have to reconnect every week, which is unacceptable for her.
- **Production but unverified:** she sees an "unverified app" warning screen. There is a lifetime cap of 100 new users for apps showing that screen (same page). It is workable but scary for a non-technical user.
- **Personal user type** (same page, current text): it limits authorization "exclusively to the single Google Account that created the application". It needs no verification, has no user cap and has **no 7-day expiry**. It is a one-time choice that cannot be changed later. This fits VFS exactly if the project is created from her account. **Unverified:** whether Personal mode is offered to consumer @gmail.com accounts (the page only discusses Workspace caveats), and whether it removes the unverified-app screen. Test it before committing.
- **YouTube API Services Developer Policies** (https://developers.google.com/youtube/terms/developer-policies), §III.E.4:
  - Authorized Data (her playlists) may be stored **no more than 30 calendar days, then deleted or refreshed**. Tokens may be kept as long as needed.
  - Non-authorized data (for example titles from an API-key `videos.list`) falls under the same 30-day limit.
  - Delete stored data within 7 days of an in-app disconnect, and within 30 days if she revokes access from Google's security settings. VFS should check token validity periodically.
  - §III.H allows YouTube to audit. Quota increases need a compliance audit, but none is needed here.
  - The API client must also have a privacy policy and accept the terms. A short static page suffices.
- Takeout data is her own export, not "API Data", so the 30-day rule does not apply to the CSVs themselves. oEmbed is a public embed endpoint that is not described in the API Services docs. Its terms status is unclear; use it sparingly (one call per new video ID, rate-limited) and cache.
- Everything stays local. No VFS server ever sees tokens or data.

### 5. Cost

- Assumptions **[estimate]**: 50 playlists × 200 items = 10,000 items.
  - `playlists.list`: 1 call = 1 unit.
  - `playlistItems.list`: 50 × ⌈200/50⌉ = 200 calls = **200 units**.
  - A full re-sync costs about **201 units**, which is 2 % of the default 10,000 units/day **[published]**.
  - Optional `videos.list` for durations or statistics: 200 more units.
  - Daily or weekly sync is free. API fees: €0.
- Takeout + oEmbed **[estimate]**: 10,000 oEmbed GETs at 1 request/s ≈ 2.8 h once, done in the background, then only new IDs.
- Storage **[estimate]**: about 1 KB per item, so ~10 MB with thumbnails as URLs. Caching thumbnails (~15 KB hqdefault) would add ~150 MB. Store URLs and fetch thumbnails only for visible results, or cache the small `mqdefault`.
- CPU/disk impact on her HDD is negligible.

### 6. Dependencies in VFS

- The multi-source model and source selector from **VFS-6 / #63** (Firefox bookmarks): a YouTube item is a sibling of a bookmark (URL, title, folder = playlist, added date).
- The OS credential-store code from **Phase 5 / #38** (reuse for the refresh token).
- Sorting by "fecha de publicación" vs "fecha de recopilación" needs two date columns on non-file items. Spec this once in VFS-6's schema.
- Thematic search ("hígado graso") over titles and descriptions uses Phase 1 FTS. Concept-level recall depends on **#40/#41**.

### 7. Smallest useful first step

A **Takeout playlist importer**:
- Parse the playlists CSV and per-playlist CSVs, tolerating Spanish or English headers.
- Store `video_id`, playlist name and added date as items in the VFS-6 source table.
- Show "Vídeo de YouTube · lista «Salud» · añadido 12/03/2024" with a link `https://www.youtube.com/watch?v=<id>`.
- Fetch titles by oEmbed as a second, optional pass.

Before writing the spec, get one real Takeout export from her account to pin the CSV format.

### 8. Recommendation

**Do next (Takeout importer), do later (OAuth live sync).** Takeout needs no Google project, no consent screen, no 7-day token trap and no 30-day deletion duty, and it is the only path to her watch history. Add OAuth sync with a **Personal**-type project only after confirming Personal mode works on her consumer account and that she actually adds to playlists often enough to need live refresh.

### 9. Open product questions for the owner

1. **Does she save videos into named playlists, or mostly into "Ver más tarde" or just watch them?** The API cannot read Watch Later or History, and that decides between Takeout and the API.
2. Is a manual re-import every few months acceptable, or must new playlist items appear automatically?
3. Should watch history (everything she watched, not saved) be searchable, or only what she deliberately saved?
4. Is it acceptable that VFS contacts youtube.com (oEmbed) to fetch titles, given the "fully offline" promise? It could be a one-off, visible step.

### 10. Sources

- https://developers.google.com/youtube/v3/docs/playlists/list
- https://developers.google.com/youtube/v3/determine_quota_cost
- https://developers.google.com/youtube/v3/revision_history
- https://developers.google.com/youtube/terms/developer-policies
- https://developers.google.com/identity/protocols/oauth2/native-app
- https://support.google.com/cloud/answer/15549945
- https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification
- https://developers.google.com/data-portability/schema-reference/youtube
- https://developers.google.com/data-portability/schema-reference/my_activity
- https://developers.google.com/data-portability/user-guide/overview
- https://github.com/iv-org/invidious/issues/4069
- https://oembed.com/ and https://www.youtube.com/oembed?url=https://www.youtube.com/watch?v=M3r2XDceM6A&format=json
- https://support.google.com/accounts/answer/3024190

<a id="line-4"></a>

## Line 4. Telegram and WhatsApp: photos, video links and contacts exchanged in chats

Issue: [#80](https://github.com/AndresHG/VFS/issues/80).

### 1. What she wants

She wants the same single search to include "fotos, enlaces de vídeo y contactos" exchanged over Telegram and WhatsApp. Examples:
- For "Todas las fotografías de localizar y quitar la fosa séptica antigua e instalar la nueva, por orden cronológico", the photos the plumber sent over WhatsApp, in date order, with who sent them and in which chat.
- For "hígado graso", a YouTube link a friend shared on Telegram, with its title.
- The phone number of a contact someone shared in a chat ("el contacto del fontanero que me pasó Pilar").

### 2. How it could work

**Option A: official exports into a watched folder plus importers (recommended).**
- **Telegram Desktop** (Windows/macOS) offers "Exportar historial del chat" per chat (⋯ menu) and "Exportar datos de Telegram" for everything (Ajustes → Avanzado), as JSON or HTML **[published]** (https://telegram.org/blog/export-and-more, 2018-08-27; https://core.telegram.org/import-export). Schema of `result.json`:
  - A full export holds `chats[]`, `contacts`, `personal_information` and more. A single-chat export is one Chat object `{id, name, type, messages[]}`.
  - Each message has `id`, `type`, `date` (ISO 8601), `date_unixtime`, `from`, `from_id`, `text` and `text_entities[]`.
  - Entity types include `link`, `text_link`, `email`, `phone`, `mention`, `hashtag`. Each entity has `type` and `text`. The documented schema shows **no `href`** for `text_link`; whether real exports carry one is unverified.
  - Media fields: `photo` and `file` (relative paths such as `chats/chat_01/photos/photo_2@29-10-2021_09-30-00.jpg`), `thumbnail`, `media_type` (video_file, voice_message, animation…), `mime_type`, `width`, `height`, `duration_seconds`.
  - A shared contact appears as `contact_information {first_name, last_name, phone_number}` plus `contact_vcard` (relative path to a `.vcf`).
  - Export options include media-type filters and a size threshold. Excluded media is replaced by a placeholder (default threshold values not verified).
- **WhatsApp** offers per-chat "Exportar chat" ("Sin archivos" / "Incluir archivos") on Android and iOS. It produces a zip with `_chat.txt` plus the media.
  - Limits **[published, third-party only]**: about 10,000 latest messages with media, 40,000 without. Older media lines become "<Media omitted>", truncated silently (https://dev.to/chattopdf/whatsapp-chat-export-complete-guide-iphone-android-zip-pdf-2026-10lo, several vendor guides). We could not load WhatsApp's own FAQ page (https://faq.whatsapp.com/1180414079177245) to confirm.
  - There is no "export all chats" button; it is one chat at a time.
  - "Solicitar info. de mi cuenta" excludes messages, per WhatsApp's own form page (https://www.whatsapp.com/contact/forms/719247716111973).
  - `_chat.txt` is plain text whose layout depends on platform and phone locale. **Illustrative examples, to be confirmed against real exports from her phone:**
    - Android, es-ES: `14/03/24, 18:05 - Pilar: IMG-20240314-WA0007.jpg (archivo adjunto)`
    - Android, en-US: `3/14/24, 6:05 PM - Pilar: IMG-20240314-WA0007.jpg (file attached)`
    - iOS, es-ES: `[14/3/24, 18:05:31] Pilar: ‎<adjunto: 00000123-PHOTO-2024-03-14-18-05-31.jpg>` (iOS inserts invisible U+200E marks)
    - iOS, en: `[3/14/24, 6:05:31 PM] Pilar: ‎<attached: 00000123-PHOTO-2024-03-14-18-05-31.jpg>`. The `<attached: …>` form is confirmed by the whatsapp-chat-parser README (https://unpkg.com/whatsapp-chat-parser@4.0.2/README.md).
    - Shared contacts arrive as `Nombre.vcf (archivo adjunto)` / `<adjunto: … .vcf>`.
  - Day/month order is ambiguous (03/04/24). The parser must infer it per file, as whatsapp-chat-parser's `daysFirst` heuristic does.
- **Importer:** the user exports chats into, for example, `Archivo\Chats\Telegram\…` and `Archivo\Chats\WhatsApp\<chat>\`, which VFS already indexes as files. A parser for `result.json` / `_chat.txt` then turns each export into three item kinds linked to the media files already in the index:
  - **Adjunto de mensaje**: path of the photo or video file plus chat name, sender, message date and surrounding caption/text. The message date beats file mtime, which is the export time, for chronological order.
  - **Enlace**: URL plus ±1 message of context, sender, chat and date. YouTube URLs get title and channel via oEmbed, shared with line 3.
  - **Contacto**: name and phone(s) from `contact_information` or the `.vcf`, plus who shared it, where and when.

**Option B: live API access.**
- Telegram via TDLib/MTProto needs an `api_id` from my.telegram.org and phone-code login **[published]** (https://core.telegram.org/api/obtaining_api_id):
  - Accounts logging in through unofficial clients are "under observation", and flooding gets permanent bans.
  - The API terms require privacy care and ban using Telegram data to train AI models (§1.5) (https://core.telegram.org/api/terms).
  - Technically feasible, but heavy (TDLib build for Windows and macOS, session secrets) and an account-risk for her.
- WhatsApp has no consumer API. The Business Platform does not apply to her personal account. **Not possible.**

**Option C: read local databases.**
- Android `msgstore.db.crypt14/15` needs the key file (root) or the 64-digit E2E backup key. An iOS backup holds `ChatStorage.sqlite` (unencrypted only if the WhatsApp E2E backup is off). WhatsApp Desktop's local store is encrypted (internals not verified).
- Tools exist (https://github.com/KnugiHK/Whatsapp-Chat-Exporter, which states it is not affiliated with WhatsApp).
- WhatsApp's EEA Terms forbid reverse engineering, decompiling or extracting code, and collecting user information "in any impermissible or unauthorized manner" (https://www.whatsapp.com/legal/terms-of-service-eea, Acceptable Use).
- Fragile across app versions, requires keys she cannot find, and conflicts with the terms. **Do not do.**

### 3. What it needs from the user

- Telegram: install Telegram Desktop, log in by QR from her phone, export (JSON, with photos, videos and files) into the agreed folder. Repeat occasionally; an export is a snapshot.
- WhatsApp: on the phone, export each important chat "Incluir archivos" and save or share the zip to the PC (or to a Synology-synced folder). This is tedious and per chat, and big chats hit the 10,000-message media cap. A helper should do the first round with her.
- The rest of her phone media is already covered: if her camera roll or the `WhatsApp/Media/WhatsApp Images` folder syncs to the PC (Synology Photos / Drive), Phase 1 file search already finds those files by name and date. They lack sender and chat context, and WhatsApp names like `IMG-20240314-WA0007.jpg` carry the date.

### 4. Privacy and terms-of-service constraints

- GDPR Art. 2(2)(c): the Regulation does not apply to processing "by a natural person in the course of a purely personal or household activity" (https://gdpr-info.eu/art-2-gdpr/, Recital 18). Indexing other people's messages, photos and phone numbers on her own PC for her own use falls under this. It stays that way only if the data never leaves the PC. Phase 5 AI features must exclude chat content by default, or show it explicitly before sending.
- Official exports are a sanctioned feature on both platforms. Parsing their output touches no service, so it raises no ToS issue.
- The Telegram API (Option B) carries account-ban risk and terms obligations. WhatsApp database reading (Option C) conflicts with the WhatsApp Terms.
- Chat exports are sensitive. VFS must not copy them elsewhere, should show them only for the "Chats" source when selected, and should let her delete one import cleanly.

### 5. Cost

- Assumptions **[estimate]**: 30 exported chats, 150,000 messages total, 20,000 media files (~40 GB), 3,000 links, 200 shared contacts.
  - **[estimate]** Parsing JSON/TXT at ~50 MB/min of text on an old 4-core CPU: the text part (~50 MB) parses in about 1 min.
  - Media files are not opened, only linked by path, so no extra HDD reads beyond Phase 1's walk.
  - Index growth: messages ~300 B each searchable → ~45 MB if all message text is indexed. Indexing only items (attachments, links, contacts) plus captions → ~10 MB.
- oEmbed for YouTube links: about 1,000 GETs once, roughly 17 min at 1/s **[estimate]**.
- Disk: exports duplicate phone media (+40 GB on the 8 TB HDD). Acceptable, but the importer could dedupe against already-synced WhatsApp Images by name + size.
- API fees: €0.

### 6. Dependencies in VFS

- Phase 1 file index: the media are already files; items reference `file_id`.
- **VFS-6 / #63** multi-source model and source selector: add a "Chats" source, with the "Enlace" item shared with bookmarks.
- Line 3's oEmbed title fetcher for YouTube links.
- Phase 3 text search would make message text searchable with BM25. Thematic queries ("fosa séptica") over photos with no caption need **#28/#41** (image understanding) or chat-context words near the photo.
- **#38** privacy gate: chat content is excluded from AI by default.

### 7. Smallest useful first step

**A Telegram `result.json` importer.** The schema is documented and stable, dates are ISO 8601, and entities are typed, so it is the lowest-risk parser. It produces attachment, link and contact items from a folder she has exported, with sender, chat and date, sortable chronologically. Then the WhatsApp `_chat.txt` parser, written against 4 real sample exports (Android/iOS × es/en) collected first as test fixtures.

### 8. Recommendation

**Do later** (after VFS-6 lands and Phase 3 is stable), **exports only**. Do not build the Telegram API, and never build WhatsApp database decryption. The value is real: chronological photos with sender context match her fosa séptica example. But the per-chat manual export, especially WhatsApp's chat-by-chat flow with its reported 10,000-message media cap (third-party sources only, see section 2), limits coverage. If her WhatsApp Images folder is synced to the PC, it already gives a cheaper partial answer through plain file search. Start with Telegram only if she actually uses Telegram Desktop or is willing to.

### 9. Open product questions for the owner

1. **Which app holds most of what she wants (the fosa séptica photos, the shared contacts), WhatsApp or Telegram, and on Android or iPhone?** That decides which parser comes first and which date format matters.
2. Is her phone's WhatsApp media folder already synced to the PC by Synology? If so, is file search plus date enough, without sender and chat?
3. Who will do the per-chat exports, and how often (one-off archive vs every few months)?
4. Should message text itself be searchable, or only attachments, links and contacts? Indexing text is more intimate and larger.

### 10. Sources

- https://telegram.org/blog/export-and-more
- https://core.telegram.org/import-export
- https://core.telegram.org/api/terms
- https://core.telegram.org/api/obtaining_api_id
- https://faq.whatsapp.com/1180414079177245 (could not be loaded; limits not confirmed from the official page)
- https://www.whatsapp.com/contact/forms/719247716111973
- https://www.whatsapp.com/legal/terms-of-service-eea
- https://dev.to/chattopdf/whatsapp-chat-export-complete-guide-iphone-android-zip-pdf-2026-10lo
- https://unpkg.com/whatsapp-chat-parser@4.0.2/README.md
- https://github.com/KnugiHK/Whatsapp-Chat-Exporter
- https://gdpr-info.eu/art-2-gdpr/
- https://www.youtube.com/oembed (shared with line 3)

<a id="line-5"></a>

## Line 5. Open-internet search alongside her own material, marking what she already has

Issue: [#81](https://github.com/AndresHG/VFS/issues/81) (closed, not planned).

> **Decided (owner, 2026-11-20): will not do.** Any LLM or agent already searches the open internet better than VFS could; VFS stays on her own material. The research below is kept for reference only and needs no further work. Its URL normalisation is still useful to deduplicate bookmarks and saved links.

### 1. What she wants

One command that searches "sus archivos, marcadores… y también internet", with results that
separate what she already has from what she does not. Example: "Necesito estudiar todo lo que
tengo sobre hígado graso" — she wants her own PDFs, notes and bookmarks first, then web
results, with web results she already bookmarked or downloaded marked as such ("ya lo tienes").

### 2. How it could work

> Reference only: line 5 is decided, will not do.

**Option A: a "Buscar también en internet" button (no API).** VFS opens
`https://<her default engine>/search?q=<query>` in her default browser (Tauri `opener` plugin).
Cost 0, integration about 20 lines **[estimate]**, no key, no new terms. She already uses her browser, so the
search engine sees nothing new. Limitation: VFS cannot see the web results, so it **cannot mark
what she already has**. That half of the request is lost.

**Option B: a search API via a key (VFS calls it directly).** VFS sends the query, gets back
JSON (URL, title, snippet, sometimes date) and shows a "En internet" block under her local
results. Each web result is matched against her bookmarks and indexed files (see "matching"
below) and tagged "Ya está en tus marcadores" / "Ya lo tienes descargado". Current providers:

| Provider | Price | Free tier | Notes |
|---|---|---|---|
| Brave Search API "Search" | $5 / 1,000 requests **[published, brave.com/search/api]** | $5 credit / month **[published]**; header also says "up to $20" (unclear) | Independent index. Storing results "with a custom plan" only. Query logs kept ≤ 90 days; ZDR on Enterprise **[published, Brave API privacy notice]** |
| Exa | $4 / 1,000 (lite/instant), $7 (auto) **[published, exa.ai/pricing]** | $10 / month, resets monthly **[published]** | ZDR only on Enterprise |
| Tavily | $0.008 / credit; basic search = 1 credit **[published, tavily.com/pricing, docs]** | 1,000 credits / month **[published]** | Built for LLM agents |
| Kagi Search API | $12 / 1,000 **[published, kagi.com/api/pricing]**; help page still says $25 (stale) | none stated | Needs a paid Kagi account |
| Mojeek | £2 / 1,000 (Startup), £3 (Business) **[published, mojeek.com]** | unstated free credit, card required | EU infra, 90-day retention default, ZDR on request; Startup may cache results 1 h only |
| SerpAPI (scrapes Google) | Free 250 / month; $25 / 1,000 / month **[published, serpapi.com/pricing]** | 250 / month | Google sued SerpApi in Dec 2025 (DMCA anti-circumvention); DMCA claims reportedly dismissed July 2026, case ongoing **[published, blog.google; Search Engine Land]**. Grey area; avoid |
| Google Custom Search JSON API | 100 free/day, $5 / 1,000 **[published]** | — | **Closed to new customers; discontinued 1 Jan 2027** **[published, developers.google.com/custom-search]**. Successor Vertex AI Search covers ≤ 50 domains, not the open web. Not an option |
| Bing Web Search API | — | — | **Retired 11 Aug 2025** **[published, Microsoft Lifecycle]**. Replacement "Grounding with Bing Search" is $14 / 1,000 and usable only inside Azure AI Foundry; outputs "not directly accessible for use in other applications" **[published, microsoft.com/bing/apis/grounding-pricing]**. Not usable by VFS |
| DuckDuckGo | — | — | No documented full web-results API (the Instant Answer API returns only zero-click summaries; we could not load DDG's help page to re-confirm) |

**Option C: the LLM provider's web search tool via the owner's key (phase 5).** Anthropic's
`web_search` server tool: **$10 / 1,000 searches plus tokens** for result content **[published,
platform.claude.com web-search-tool]**. Results include `url`, `title`, `page_age`, so matching
works. ZDR-eligible (excluding dynamic filtering). Citations must be shown when outputs are
displayed. OpenAI web search tool: **$10 / 1,000 calls**, search content tokens billed at model
rate **[published, developers.openai.com/api/docs/pricing]**. Advantage: same key and consent
screen as phase 5; the model can also rewrite her thematic query ("hígado graso" → "esteatosis
hepática no alcohólica dieta") and summarise. Drawback: needs the phase-5 module, costs tokens,
and the provider (and its upstream search vendor) sees the query.

**Matching web results against her material** (needed for B and C):
- *URL normalisation* (pure Rust, `url` crate): lowercase scheme and host; drop `www.` and
  `m.`/`mobile.` host prefixes; drop default ports, fragment and trailing slash; drop tracking
  params (`utm_*`, `fbclid`, `gclid`, `mc_cid`, `igshid`, `si`); sort remaining params; treat
  http/https as equal. Site rules: `youtu.be/ID`, `youtube.com/watch?v=ID`, `/shorts/ID`,
  `/embed/ID`, `m.youtube.com` → `youtube:ID`; `amazon.*/dp/ASIN` and `/gp/product/ASIN` →
  `amazon:ASIN`; `doi.org/10.x` and `dx.doi.org` → `doi:10.x`. Store the normalised key next to
  each bookmark at VFS-6 index time; matching is then one indexed lookup per result.
- *Exact match* against bookmarks (VFS-6) and, later, saved-link items from other sources.
- *Title fuzzy match* against local file names: downloaded PDFs are often named after the
  article title. Accent-fold and lowercase both, drop stopwords, compare with the trigram
  similarity that phase 1 already uses; flag "posiblemente ya lo tienes" above a threshold
  (tune on her data), never "ya lo tienes".
- *DOI match*: when a web result URL or snippet contains a DOI (Crossref regex, line 8), compare
  with DOIs extracted from her PDFs (phase 3 / line 8).

### 3. What it needs from the user

> Reference only: line 5 is decided, will not do.

- A: nothing.
- B: an API account and key per provider (the owner creates it; credit card for most), stored
  in the OS credential store like the phase-5 key.
- C: the phase-5 Anthropic/OpenAI key.
- For B/C: an explicit opt-in ("Buscar también en internet"), off by default.

### 4. Privacy and terms-of-service constraints

> Reference only: line 5 is decided, will not do.

- Her queries are health-revealing ("hígado graso", "colágeno"). With B/C they leave the PC
  tied to an API key that identifies the owner's account.
- Retention: Brave API query logs ≤ 90 days **[published]**; Mojeek 90 days default
  **[published]**; Anthropic API inputs/outputs deleted within 30 days by default **[published,
  privacy.claude.com]**; OpenAI abuse-monitoring logs up to 30 days, and web search is not in
  its ZDR table **[published, developers.openai.com your-data]**. Exa/Tavily: no default
  retention statement found.
- UI rule (matches phase-5 design): off by default; one-time consent explaining which company
  receives the query; before each web search show the exact text being sent ("Se enviará a
  Brave: «hígado graso»"); never send file names, paths or snippets of her documents as part of
  a query unless she types them.
- ToS: Brave standard plans forbid storing results without a custom plan, so VFS must not
  persist web results in its index (show only, re-query on demand). Anthropic requires showing
  citations. Avoid SerpAPI (scraping, litigation).

### 5. Cost

> Reference only: line 5 is decided, will not do.

Assume 10–30 web searches/day ≈ 300–900/month **[estimate]**.
- A: $0.
- Brave: 300–900 × $5/1,000 = $1.50–4.50/month, inside the $5 monthly credit → **≈ $0** **[estimate]**.
- Tavily: ≤ 1,000 basic searches/month free → **$0**; Exa: $10 credit covers it → **$0** **[estimate]**.
- Kagi: $3.60–10.80/month; Mojeek: £0.60–1.80/month **[estimate]**.
- Anthropic web search: $3–9/month for searches, plus tokens. With Haiku 5.5 ($0.10/MTok input
  **[published]**) and ~10k tokens of results per search: ~$0.001/search → ~$1/month at 900
  searches **[estimate]**; larger models cost proportionally more.
- Target PC: negligible (one HTTPS call, JSON parse, a few hundred indexed lookups).

### 6. Dependencies in VFS

> Reference only: line 5 is decided, will not do.

VFS-6 / #63 (bookmarks with stored normalised URL), phase 1 (fuzzy title match), phase 3 (DOIs
in PDFs, optional), phase 5 #38 (key storage, consent and "what leaves the PC" screen; Option C
needs #38/#40). The unified "one command, several sources" result list is shared with other lines.

### 7. Smallest useful first step

None: decided, will not do. The research found that the cheapest form would have been a
«Buscar también en internet» button opening her browser; the owner judged that any LLM or
agent already covers this better. The URL normalisation in section 2 is still useful to
deduplicate bookmarks and saved links within VFS.

### 8. Recommendation

**Decided (owner, 2026-11-20): will not do.** Any LLM or agent already searches the web better
than VFS could. Kept for reference: had it been built, Brave Search would have been the
provider (free at her volume, documented 90-day retention), and SerpAPI, Google Custom Search
and Bing were to be avoided.

### 9. Open product questions

None now: the owner has decided.

### 10. Sources

- https://brave.com/search/api/
- https://api-dashboard.search.brave.com/privacy-policy
- https://developers.google.com/custom-search/v1/overview
- https://learn.microsoft.com/en-us/lifecycle/announcements/bing-search-api-retirement
- https://www.microsoft.com/en-us/bing/apis/grounding-pricing
- https://platform.claude.com/docs/en/agents-and-tools/tool-use/web-search-tool
- https://platform.claude.com/docs/en/about-claude/pricing
- https://privacy.claude.com/en/articles/7996866-how-long-do-you-store-my-organization-s-data
- https://developers.openai.com/api/docs/pricing
- https://developers.openai.com/api/docs/guides/your-data
- https://www.tavily.com/pricing ; https://docs.tavily.com/documentation/api-credits
- https://exa.ai/pricing
- https://kagi.com/api/pricing ; https://help.kagi.com/kagi/api/search.html
- https://www.mojeek.com/services/search/web-search-api/
- https://serpapi.com/pricing
- https://blog.google/technology/safety-security/serpapi-lawsuit/
- https://searchengineland.com/google-loses-key-dmca-claims-against-serpapi-in-scraping-lawsuit-483185

<a id="line-6"></a>

## Line 6. Sorting by publication date, collection date and source

Issue: [#82](https://github.com/AndresHG/VFS/issues/82).

### 1. What she wants

Results "sortable by publication date, collection date and source". Example 6: "Todas las
fotografías de localizar y quitar la fosa séptica antigua e instalar la nueva, por orden
cronológico". Two different dates matter:
- **published_at**: when the work appeared (an article from 2014, a video uploaded in 2019).
- **collected_at**: when *she* got it (downloaded, bookmarked, added to a playlist, received on
  WhatsApp, photo taken).

For photos the two collapse into "when it was taken", which is what example 6 needs.

### 2. How it could work

**Where each date comes from, per source:**

| Source | published_at | collected_at | Reliability |
|---|---|---|---|
| Local files: filesystem | none | mtime ≈ last edit or download; NTFS creation time; macOS `st_birthtime` (APFS/HFS+) | mtime: medium. Windows copies keep mtime but set creation time to the copy time, so creation time on a copied archive means "when it reached this disk". Same-volume moves keep it. |
| PDF | Info `CreationDate`, XMP `xmp:CreateDate`/`dc:date` | n/a | Low as publication date: usually when the PDF was *generated* (re-export, scanner, "print to PDF") |
| OOXML (docx/xlsx/pptx) | `docProps/core.xml` `dcterms:created` | n/a | Authoring date of the file, copied forward from templates. Low as publication date. |
| EPUB | `dc:date` = "publication date of the EPUB publication… not the same as the last modified date" | n/a | Medium: the edition's date, not the original work's |
| Photos and video | EXIF `DateTimeOriginal` (0x9003) plus `OffsetTimeOriginal` (0x9011) when present; video `creation_time` | same as taken | High for camera and phone originals (no time zone without the offset tag). WhatsApp-recompressed images usually lose EXIF and fall back to file time or the name pattern `IMG-20190612-WA0003.jpg`. |
| Firefox bookmarks | none in places.sqlite | `moz_bookmarks.dateAdded`, PRTime = µs since 1970 (divide by 1,000,000) | High. Imported bookmarks may carry the import date instead of the original. |
| YouTube playlists (API; Takeout gives only the added date) | `contentDetails.videoPublishedAt` | `snippet.publishedAt` = "date and time that the item was added to the playlist" | High for both |
| Telegram Desktop export (JSON) | forwarded original date, if exported | message `date` (ISO 8601) and `date_unixtime` | High (`edited` key unverified) |
| WhatsApp export (.txt) | none | line-prefix timestamp, format depends on phone locale and OS (`12/06/19 18:03 -` vs `[12/6/19, 6:03:21 p. m.]`) | Medium: day/month ambiguity must be resolved per file. Exports reportedly cap at ~40k messages without media, ~10k with media (third-party guides, not confirmed by WhatsApp). |
| Bookmarked pages (only if a page were fetched, which is not planned) | schema.org `datePublished` ("date of first publication"), OG `article:published_time`, HTTP `Last-Modified` | when fetched or bookmarked | JSON-LD/OG: medium-high on news and blogs, absent elsewhere. `Last-Modified` is low (often the render time on CMS pages). |

Sources in §10. Synology Drive: **we could not find official documentation** on whether Drive
Client preserves mtime or creation time. Community reports say sync tools on Windows commonly
reset "Date created" on downloaded files while mtime usually survives, and one user reports
Drive rewriting timestamps. **Treat creation time on her disk as untrustworthy** and verify
during the target-PC measurement plan ([target-PC measurement plan](target-pc-measurement-plan.md)): stat a known file before and after a Drive
round-trip.

**Option A: file mtime only (status quo plus sort).** Phase 1 already stores mtime and filters
by date (VFS-1 R8). Adding a sort and showing it as "Fecha" is trivial, but it is ambiguous
(edited vs downloaded).

**Option B: a normalised two-date model (recommended).** Per item:
```
published_at       INTEGER NULL   -- unix seconds, UTC
published_src      TEXT           -- 'exif','youtube','schema_org','og','epub','pdf_info','ooxml','http_lm'
published_conf     INTEGER        -- 0 unknown, 1 low, 2 medium, 3 high
collected_at       INTEGER NULL
collected_src      TEXT           -- 'mtime','ntfs_ctime','birthtime','ff_dateAdded','yt_added','tg_msg','wa_line'
collected_conf     INTEGER
precision          TEXT           -- 'day','second' (WhatsApp/EPUB often day-only)
```
Each source fills the fields it knows. A per-source precedence list picks the best candidate
(for photos: EXIF > filename pattern > mtime). Low-confidence values are kept but shown with a
"≈" marker. Sorting: `ORDER BY published_at IS NULL, published_at DESC`, with unknown dates
always last and counted ("37 sin fecha").

**Option C: also infer dates from content** (a year in the file name or folder like
`2019/fosa/`, a date in the PDF's first page). It is useful for her year folders but heuristic.
Use it as a low-confidence fallback only.

**UI.** A sort control: "Más reciente / Más antiguo", then "por fecha de publicación / por
fecha en que lo guardé", then a group or filter by source ("Mis archivos · Marcadores ·
YouTube · Telegram · WhatsApp"). Each result shows its date with the source in plain
words on hover ("Fecha de la foto (cámara)", "Añadido a marcadores"). Example 6 is "Fotos" +
"fosa séptica" + sort ascending by taken date, but the photos have no names (example 4 problem),
so finding them needs image content or folder context (another line). This line only orders
them.

### 3. What it needs from the user

Nothing for files and bookmarks. For YouTube: a Takeout export, the first step of
[line 3](#line-3); Takeout gives only the date she added each video, so a publication date
needs the API (OAuth, the later step of line 3). For Telegram: a Desktop JSON export. For WhatsApp: per-chat
exports from the phone. Optionally she confirms whether WhatsApp dates are day/month.

### 4. Privacy and terms-of-service constraints

All extraction is local. Reading EXIF and document metadata is in-process parsing, done in the
phase 3 isolated worker for untrusted files. Publication dates that need fetching a web page
are not planned: open-internet search (line 5) was dropped, and VFS stays offline by default. The YouTube Data API
returns both dates under the normal API terms.

### 5. Cost on the target PC and/or API fees

- Two date pairs plus src/conf: ~20–30 bytes/row, so ~150 MB for 5 M rows **[estimate]**. A
  second-level index per sortable date adds ~100 MB **[estimate]**. Acceptable, but measure
  against the phase 1 footprint budget.
- EXIF needs only the first 64 KB of each JPEG/HEIC. For ~500k photos (a deliberately high count; line 2 assumes 300,000) that is ~32 GB of reads,
  about 2–3 h of sequential-ish reads on a busy HDD, more if seeks dominate **[estimate:
  50–100 random reads/s, 500k files → 1.5–3 h]**. It breaks phase 1's "never opens files" rule,
  so it is a phase 3-style background pass. NTFS creation time comes free with the directory
  enumeration Phase 1 already does (`FindFirstFileEx` returns it), so no extra I/O.
- No API fees except YouTube quota (free tier, 1 unit per playlistItems page).

### 6. Dependencies in VFS

Phase 1 schema (mtime already stored; adding the creation time is cheap). VFS-6 bookmarks
(`dateAdded`). Phase 3 worker (PDF/OOXML/EPUB/EXIF metadata). The YouTube, Telegram and WhatsApp
source lines. The target-PC measurement plan for Synology timestamp behaviour.

### 7. Smallest useful first step

**Decided (owner, 2026-11-20): reasonable; the line most likely to be done soon.** The first
step below is written to be specified (`/spec`, size M) and built without more research. It
is based on `main` at commit `3af9d6e` (phase 1 and the «Papel» search screen).

**What exists today.** `SearchQuery` (`crates/vfs-core/src/types.rs`) has filters, including a
modified-date range, but no sort field. `rank.rs` orders hits by relevance tiers, then name
length, then newest `modified`, then id; a filter-only query is newest first. The search ranks
at most 2,000 hits (`MAX_HITS` in `search.rs`) and returns a page `[offset, offset + limit)`.
Each `SearchHit` already carries `modified` (unix seconds). Folders have `mtime = 0` until their
parent is listed. The UI shows the date per row, with «modificado el …» in the row's accessible label (`apps/desktop/ui/strings.js`) and has
a date filter but no sort control. Firefox bookmarks (VFS-6, issue #63) are not merged yet.

**Change.**

1. Core: add `sort: Relevance | NewestFirst | OldestFirst` to `SearchQuery`, default
   `Relevance` through serde, so existing callers and stored queries keep working.
2. Core: `NewestFirst` / `OldestFirst` reorder the **whole ranked list** (≤ 2,000 hits) by the
   hit's collected date before the page is cut, so paging is stable. Ties keep relevance
   order. A date of 0 (unknown, such as an unlisted folder) sorts **last in both directions**.
   The sort also applies to hits served from the search cache (`search.rs`), before paging.
   No new SQL, index or schema change: the dates are already on the hits.
3. When the search was truncated (`truncated_by` set), the result line says that the order
   covers the first 2,000 matches, so «Más antiguos» is not mistaken for the oldest file on
   the disk.
4. UI: a sort selector next to the filters, «Relevancia» (default), «Más recientes»,
   «Más antiguos». It is kept for the session and reset to «Relevancia» on restart. The row's
   date label follows the source: «modificado el …» for files and folders, «añadido el …» for
   bookmarks.
5. With VFS-6: a bookmark hit's collected date is `moz_bookmarks.dateAdded` (µs → s), and
   each hit carries its source. In the combined view each row shows a source label
   («Archivo», «Carpeta», «Marcador»), and the selector sorts files and bookmarks together.
   Sorting *by source* is the source selector plus this label; a separate «por fuente» order
   is not needed.

**Acceptance criteria.**

- AC-1: With «Más recientes», hits are in descending collected date; with «Más antiguos»,
  ascending; in both, hits with an unknown date come after all dated hits.
- AC-2: Equal dates keep the relevance order of the default sort (deterministic for any input
  order).
- AC-3: The order is computed over the whole ranked list before paging: concatenating pages
  0, 1, 2… gives the same sequence as one large page.
- AC-4: A query without `sort` behaves exactly as today (existing ranking tests unchanged).
- AC-5: A truncated search sorted by date shows that the order covers the first 2,000 matches.
- AC-6: The selector offers the three Spanish labels, defaults to «Relevancia», and survives a
  new query in the same session.
- AC-7 (with VFS-6): files and bookmarks sort together by collected date; each row shows its
  source label and «modificado» or «añadido» as fits.
- AC-8: Sorting adds no measurable latency: under 5 ms for 2,000 hits on the benchmark
  machine **[estimate: an in-memory sort of 2,000 keys]**.

**Tests.** Unit tests in `rank.rs` (or the new sort function) for AC-1 to AC-4; a core search
test for paging (AC-3) and truncation (AC-5); UI tests in `apps/desktop/ui-dev` for AC-6
(on the «Papel» search screen merged in PR 64, which still has no sort control); a
combined-view test once VFS-6 lands (AC-7). Existing `rank.rs` and `tests/search.rs` tests
must pass unchanged.

**Out of this step.** Publication dates; NTFS creation time and macOS `st_birthtime` (store
them only after the target-PC measurements show whether Synology preserves them; a schema
change other sessions own); EXIF dates (photos are found by name and folder only, by the owner's decision); dates from other
sources, which arrive with their importers.

### 8. Recommendation

**Do next**: the sort selector above, files first, bookmarks as soon as VFS-6 lands. **Do
later:** the normalised two-date model (`published_at`, `collected_at`, each with source and
confidence), filled per source as each source line lands. **Do not do:** treat PDF/Office
creation dates as publication dates without a "≈" low-confidence marker. They are mostly
authoring or export dates.

### 9. Open product questions for the owner

1. **For her, does "fecha" by default mean when the thing was published or when she saved it?**
   That decides the default sort and the label.
2. Should items with only a low-confidence date (≈) sort among dated items or with the unknowns?
3. For photos without EXIF, is a year folder (`2019/`) good enough as the date?

### 10. Sources

- https://learn.microsoft.com/en-us/answers/a/7791840 ("Date created" = first copied or saved
  to that volume); https://codesector.com/how-to/copy-files-and-keep-timestamps
- https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfiletime
- https://forums.spacerex.co/t/preserving-timestamps-on-dsm-seeking-best-practices-from-archivists-or-metadata-conscious-users/3037
  (anecdotal: Drive rewrites timestamps)
- https://kb.synology.com/da-dk/DSM/help/SynologyDriveClient/synologydriveclient?version=7 (no
  timestamp documentation)
- https://www.w3.org/TR/epub-33/ (§5.5.3.2.4 dc:date, §5.5.5 dcterms:modified)
- https://exiftool.org/TagNames/EXIF.html (0x9003, 0x9011)
- https://bugzilla.mozilla.org/show_bug.cgi?id=1110042 ;
  https://forensics.wiki/mozilla_firefox_3_history_file_format (PRTime µs)
- https://searchfox.org/firefox-main/source/toolkit/components/places/nsPlacesTables.h
- https://developers.google.com/youtube/v3/docs/playlistItems
- https://core.telegram.org/import-export
- https://dev.to/chattopdf/whatsapp-chat-export-complete-guide-iphone-android-zip-pdf-2026-10lo
  (third-party; WhatsApp FAQ page would not render)
- https://schema.org/datePublished ; https://ogp.me/
- PDF Info/XMP and OOXML core properties: ISO 32000 §14.3 and ECMA-376 Part 2 (from knowledge,
  not re-fetched)

<a id="line-7"></a>

## Line 7. Reorganising about 3,000 Firefox bookmarks by a criterion she states

Issue: [#83](https://github.com/AndresHG/VFS/issues/83) (closed, not planned).

> **Decided (owner, 2026-11-20): will not do for now.** Product invariant: VFS is purely consultative and never operates on, modifies or writes to any source, Firefox included. Every way found below to apply a new structure (HTML import, JSON restore, a WebExtension calling `bookmarks.move`) writes to Firefox and is excluded by that invariant. The research is kept for reference. Read-only parts (duplicates by normalised URL, counts per domain, dead-link reporting) could return as search features over VFS-6 bookmarks without touching Firefox.

### 1. What she wants

"Reorganizar los marcadores según el criterio que yo diga", e.g. "por temas: salud, cocina,
música, la casa…". She has ~3,000 bookmarks, presumably in a few large or messy folders. She
wants Firefox itself to end up organised, not just a view inside VFS.

### 2. How it could work

> Reference only: line 7 is decided, will not do for now; any write to Firefox is excluded.

The approaches split into two parts: **deciding** the new folder for each bookmark, and
**applying** that change to Firefox safely.

**Deciding — three options**

- **D1. No model (deterministic).** Group by domain (youtube.com, a recipe site), by existing
  folder, by keyword rules she can edit ("receta|horno|harina" → Cocina), plus TF-IDF clustering
  of titles (optionally enriched with fetched `<title>`/`og:description`). Also flag duplicates
  by normalised URL (same normaliser as line 5) and dead links (below). Cheap, offline,
  explainable. But keyword rules do not "understand" a criterion stated in her own words, and
  TF-IDF clusters on 5–10-word Spanish titles are noisy.
- **D2. Local multilingual embedding model + clustering.** E.g. a small multilingual sentence
  model (~100–500 MB) embeds title + URL path + description; cluster with HDBSCAN or k-means
  (k = number of folders she names), or assign each bookmark to the nearest folder-name
  embedding (zero-shot). Offline, no fees. Costs: model download and RAM on an 8 GB PC, and
  folder labels still need a human. It overlaps with #41 (embeddings: evaluate) and #28 and
  should reuse whatever that evaluation picks.
- **D3. LLM via the owner's key (phase 5).** Send title + normalised URL (+ description if
  fetched) in batches of ~50, along with the folder scheme she dictates ("por temas de salud,
  cocina, música…"). The model returns `{id → folder}` plus "No sé" for unclear items. A
  first optional pass sends all titles (~60k tokens) and asks the model to *propose* a scheme
  from her criterion, which she then edits. This handles her phrasing best. It sends bookmark
  titles and URLs (which reveal health interests) to the provider.

**Applying (all excluded by the consultative invariant; kept as research)**

- **Never write `places.sqlite`.** Firefox holds locks on it while running, and readers get
  "database is locked" **[published, SQLite forum, Mozilla bugs 627936/1285041]**. Writing
  behind Firefox's back would also bypass Sync's change tracking. VFS-6 already reads a copy
  only; keep that.
- **Excluded by the invariant (writes to Firefox).** **HTML import (Library → Import and Backup → Import Bookmarks from HTML)** **adds**; it never
  replaces or de-duplicates. Imported items go under the Bookmarks Menu (often an "Imported"
  folder) **[published, Mozilla support; Bugzilla 1353217: import "creates new bookmarks
  blindly… never de-dupes"]**. For a reorganisation this means the old copies remain, she
  would have to delete them by hand, and Sync spreads the duplicates to other devices. HTML
  import is good for **adding** a new "Marcadores reorganizados" tree alongside the old one,
  not for moving.
- **Excluded by the invariant (writes to Firefox).** **Restore from JSON (Library → Import and Backup → Restore → Choose File)** **replaces all**
  bookmarks **[published, Mozilla support "Restore bookmarks from backup", which recommends HTML
  import to merge instead]**. VFS could write a complete reorganised tree in Firefox's JSON
  backup format. This is the only no-extension way to get a true *move*, but it is
  all-or-nothing: an error drops bookmarks, and it interacts with Sync (Firefox 57 stopped HTML
  imports from forcing a Sync reset; restore behaviour under Sync must be tested). It needs
  Firefox closed or a careful user flow.
- **Excluded by the invariant (writes to Firefox).** **WebExtension using `browser.bookmarks.move()`** (requires the `bookmarks` permission).
  Moves are real, incremental, Sync-aware and keep GUIDs, tags and keywords. VFS writes a move
  plan file and the extension applies it. Cost: release Firefox requires Mozilla signing; an
  **unlisted (self-distributed)** add-on gets automated validation (up to 24 h) and may be
  manually reviewed later **[published, extensionworkshop.com]**. She would install it once
  from a file, and the owner maintains it on AMO. More moving parts; the safest of the write paths, all of which are excluded.

**Undo (relevant only to the excluded write paths)**: Firefox keeps automatic backups in `<profile>/bookmarkbackups/bookmarks-YYYY-MM-DD_
<count>_<hash>.jsonlz4`, 15 by default (`browser.bookmarks.max_backups`), written when bookmarks
changed **[published, Mozilla support threads, ghacks]**. Since VFS never applies changes,
VFS itself needs no undo.

**Dead-link check (opt-in, separate button)**: HEAD request, falling back to GET (many servers
reject HEAD) with 10 s timeout, ≤ 2 redirects followed and recorded, per-host limit of 1 request
at a time and ~1/s, global concurrency 8. Classify: 404/410 = dead; DNS NXDOMAIN or TLS failure =
dead site; 401/403/429/5xx = "no comprobable" (bot blocks are common, never call these dead);
200 that redirects to the home page or has "no encontrada/not found" in the title = soft 404
(only "posible"). For dead ones, query the Wayback Availability API
`https://archive.org/wayback/available?url=<url>`, which returns the closest snapshot URL and
timestamp, or an empty object **[published, archive.org/help/wayback_api.php; no rate limit
documented, page last updated 2013]**. YouTube: use the oEmbed endpoint to detect removed or
private videos instead of loading pages.

### 3. What it needs from the user

> Reference only: line 7 is decided, will not do for now; any write to Firefox is excluded.

- Her criterion in words, for an organised *view* inside VFS. Applying it to Firefox (closing
  Firefox for a JSON restore, installing an extension, an HTML import) is excluded by the
  consultative invariant, and would in any case be steps a non-technical user can get wrong.
- For D3: the phase-5 key and consent showing a sample of what is sent.
- For the dead-link check: explicit opt-in.

### 4. Privacy and terms-of-service constraints

> Reference only: line 7 is decided, will not do for now; any write to Firefox is excluded.

- D1/D2 are local. D3 sends ~3,000 titles + URLs to Anthropic/OpenAI (30-day default retention,
  line 5 sources). Strip query strings and fragments before sending (they can contain tokens or
  session ids); send host + path + title only.
- Dead-link checks and description fetching reveal each bookmarked URL to its site and her IP to
  ~1,000 hosts. Opt-in only, with a plain warning, a polite User-Agent with no personal data, and
  no cookies.
- Firefox Sync: one more reason the write paths are excluded; any change applied to Firefox would
  propagate to her other devices.
- Wayback API: free public service; keep requests low (only for confirmed-dead links).

### 5. Cost

> Reference only: line 7 is decided, will not do for now; any write to Firefox is excluded.

- D3 tokens **[estimate]**: 3,000 bookmarks × ~80 input tokens + 60 batches × ~500 tokens of
  instructions ≈ 0.27 M input; ~15 output tokens per bookmark ≈ 0.045 M output.
  - Claude Haiku 5.5 ($0.10 / $0.50 per MTok **[published]**): ≈ $0.05; Batch API (−50 %): ≈ $0.03.
  - Claude Haiku 4.5 ($1 / $5): ≈ $0.50; batch ≈ $0.25.
  - gpt-5.4-mini ($0.75 / $4.50 **[published]**): ≈ $0.41; gpt-5-nano ($0.05 / $0.40): ≈ $0.03.
  - With fetched descriptions (~+100 tokens each) input roughly doubles: still under $1.
  - Optional scheme-proposal pass (60k tokens in, 2k out): < $0.01 on Haiku 5.5.
- D2 on the target PC **[estimate]**: a ~120 M-parameter multilingual encoder on an older 4-core
  CPU, ~20–50 short texts/s → 1–3 min for 3,000 titles, plus ~0.5–1 GB RAM while running.
  Unmeasured; depends on #41.
- Dead-link check **[estimate]**: 90 % of URLs answer in ~1.5 s, 10 % hit the 10 s timeout →
  mean 2.35 s; 3,000 × 2.35 / 8 concurrent ≈ 15 min. A host with many links (e.g. 500 YouTube
  links at 1/s) takes ~8 min and runs in parallel, so 15–25 min in total, mostly network-bound.
  Negligible CPU/disk.

### 6. Dependencies in VFS

> Reference only: line 7 is decided, will not do for now; any write to Firefox is excluded.

VFS-6 / #63 bookmarks reader (folder tree, GUIDs, normalised URL). Phase 5 #38 for D3. #41 for
D2. The URL normaliser described in line 5 (kept as a shared piece of the source model). The
WebExtension that applying moves would need is excluded.

### 7. Smallest useful first step

None: decided, will not do for now. Under the consultative invariant, the only parts that
could return are read-only search features over VFS-6 bookmarks, shown inside VFS and never
applied to Firefox: duplicates by normalised URL, counts per domain and per folder, and an
opt-in dead-link report with Wayback links.

### 8. Recommendation

**Decided (owner, 2026-11-20): will not do for now.** VFS is purely consultative and never
writes to a source. Every way of applying a new structure (HTML import, JSON restore, a
WebExtension calling `bookmarks.move()`) writes to Firefox and is excluded. The classification
research (D1–D3 and its costs) is kept in case the owner wants an organised *view* inside VFS
later, which would not touch Firefox.

### 9. Open product questions

None now: the owner has decided. If the read-only view is ever considered: is it acceptable to
send bookmark titles and URLs to an AI provider for classification?

### 10. Sources

- https://support.mozilla.org/kb/restore-bookmarks-from-backup-or-move-them
- https://support.mozilla.org/en-US/kb/import-bookmarks-html-file/revision/27187
- https://bugzilla.mozilla.org/show_bug.cgi?id=1353217
- https://support.mozilla.org/questions/1148206 (import appends; Sync does not duplicate)
- https://support.mozilla.org/questions/1247827 ; https://www.ghacks.net/?p=102106 (backups, max_backups = 15)
- https://bugzilla.mozilla.org/show_bug.cgi?id=627936 ; https://bugzilla.mozilla.org/show_bug.cgi?id=1285041
- https://sqlite.org/forum/forumpost/86a67f6995
- https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/bookmarks/move
- https://extensionworkshop.com/documentation/publish/signing-and-distribution-overview/
- https://archive.org/help/wayback_api.php
- https://platform.claude.com/docs/en/about-claude/pricing ; https://developers.openai.com/api/docs/pricing

<a id="line-8"></a>

## Line 8. Bibliographic list with cover images

Issue: [#84](https://github.com/AndresHG/VFS/issues/84).

### 1. What she wants

"Una lista bibliográfica (con imágenes de las portadas) sin tener que teclearlo en Excel." For
example, all the books and papers she has on "solmisación" or "hígado graso", with title,
author, year, publisher and cover, ready to print or keep. The phrase "sin teclearlo en Excel"
suggests she currently builds such lists by hand in Excel, so **an XLSX with one row per work and
a cover in a cell may be the real deliverable**, plus a printable version.

### 2. How it could work

**Step 1 — identify works (local, offline):**
- **EPUB**: read the OPF package. `dc:identifier` (often `urn:isbn:` or a bare ISBN),
  `dc:title`, `dc:creator`, `dc:date`, `dc:publisher`, `dc:language`. EPUB 3 requires only
  identifier, title and language; creator and date are optional **[published, W3C EPUB 3.3
  §5.5.3]**. Cover: the manifest item with `properties="cover-image"` (EPUB 3, §D.6.1), or
  `<meta name="cover" content="<item id>">` (EPUB 2). Covers are free and local.
- **PDF**: Info dictionary (`/Title`, `/Author`, often junk like "Microsoft Word - doc1") and
  XMP (`dc:title`, `dc:creator`, `prism:doi`, `pdfx:doi`). Then text of the first ~5 pages (the
  copyright page; phase 3 extracts it): ISBN regex
  `(?:ISBN(?:-1[03])?:?\s*)?(97[89][- ]?)?\d{1,5}[- ]?\d+[- ]?\d+[- ]?[\dX]` normalised to digits,
  **validated by checksum** (ISBN-13: weights 1,3 sum mod 10 = 0; ISBN-10: weights 10…1 mod 11,
  X = 10) to reject phone numbers and page counts. Spanish ISBNs start 978-84. DOI: Crossref's
  recommended pattern `10.\d{4,9}/[-._;()/:A-Z0-9]+` (case-insensitive) matches 99.3 % of
  Crossref DOIs **[published, Crossref blog]**; trim trailing punctuation. Cover: render page 1
  as a thumbnail (pdfium/mupdf in the phase-3 worker).
- **Bookmarks** (VFS-6): URL patterns for `doi.org/…`, publisher DOI paths, `amazon.*/dp/<ASIN>`
  (ASIN = ISBN-10 for books), `casadellibro.com/libro-…/<ISBN13>/…`, `books.google.*/books?id=`,
  `openlibrary.org/isbn/…`, `goodreads.com/book/show/…`.
- **File names** like "Autor - Título (2015).pdf": weak signal, use as a fallback only.

**Step 2 — enrich (optional online lookup, ISBN/DOI only):**
- **Open Library** Books API (`/isbn/<isbn>.json`, `/api/books?bibkeys=ISBN:…`): 1 req/s
  anonymous, 3 req/s with a `User-Agent` containing an app name and contact **[published,
  openlibrary.org/developers/api]**. Covers: `https://covers.openlibrary.org/b/isbn/<isbn>-M.jpg`
  with `?default=false` to get 404 instead of a blank image; lookups by ISBN are limited to **100
  requests / IP / 5 min** (403 beyond) **[published, openlibrary.org/dev/docs/api/covers]**; asks
  not to bulk-download. Data: Internet Archive "does not assert any new copyright" (not literally
  CC0) **[published, openlibrary.org/developers/licensing]**. Spanish-book coverage is patchy.
- **Crossref REST API** (`/works/<doi>`) for papers: public pool 5 req and concurrency 1; polite
  pool (add `mailto=`) 10 and concurrency 3 **[published, Crossref access-and-authentication]**.
  Bibliographic metadata is facts / effectively free to use; abstracts stay under publisher
  copyright **[published, Crossref REST API metadata licence page]**. No covers for papers, so
  use a page-1 render.
- **Google Books API** (`q=isbn:<isbn>`): requires an API key; `imageLinks.thumbnail` gives
  covers **[published, developers.google.com/books/docs/v1/using]**. Default quota commonly cited
  as 1,000/day (not on an official page we could find; a 2026 report shows 20 M) — verify in the
  Cloud console. Terms forbid charging users a fee for the app **[published, Books API terms]**.
  Best coverage of Spanish trade books.
- **Base de datos de libros editados en España** (Ministerio de Cultura, Agencia del ISBN): a
  public web search by ISBN/title **[published, cultura.gob.es/webISBN]**. No API or reuse terms
  found, so it is a manual "ver ficha" link, not something to scrape.
- **DILVE** (publishers' ONIX catalogue): access requires credentials; no public API documented.
  Not usable.
- **datos.bne.es** (Biblioteca Nacional): linked open data under CC0 with a SPARQL endpoint
  **[published, bne.es linked-data page]**; ISBN querying not documented (needs a test). Good
  for older Spanish books; no covers.
- **ISBNdb**: paid (secondary sources: plans ~$15–300/month; official page returned 403).
  Not needed at her scale.

**Step 3 — output (three options, not exclusive):**
- **O1. XLSX with covers** via `rust_xlsxwriter` (0.103.0): `Worksheet::embed_image()` places
  the picture *in* the cell (Excel "Place in Cell"), but needs **Excel 365 from 2023 onwards**;
  older Excel shows `#VALUE!`. `insert_image_fit_to_cell()` floats the image over the cell and
  works in all Excel versions **[published, docs.rs rust_xlsxwriter]**. PNG/JPG/GIF/BMP only (no
  WebP), so convert covers to JPEG at ~150 px height. Columns: Portada, Título, Autor, Año,
  Editorial, ISBN/DOI, Tipo, Dónde está (path with hyperlink), Fuente de datos. Default to
  `insert_image_fit_to_cell` unless we know she has Excel 365; test in LibreOffice too.
- **O2. Printable HTML → PDF** with thumbnails and a citation line per item, generated locally
  and opened in the browser for "Imprimir / Guardar como PDF". No new dependencies.
- **O3. Citation formats**: CSV, BibTeX, RIS, CSL-JSON (simple serialisers). For formatted
  citations, `citeproc-rs` is **archived (Aug 2026)** and was never on crates.io **[published,
  GitHub]**; **hayagriva** (Typst) is a maintained Rust CSL processor that supports the official
  CSL styles **[published, docs.rs/hayagriva]**, or use a fixed APA-like Spanish template
  ("Apellido, N. (Año). *Título*. Editorial.") with no dependency.

### 3. What it needs from the user

- A selection: a search result set or folder ("todo lo que tengo sobre solmisación") → "Crear
  lista bibliográfica".
- Optional consent to online lookups ("Buscar datos y portadas en internet (solo se envía el
  ISBN o DOI)"). No accounts for Open Library/Crossref; a Google Books API key held by the owner
  if Google Books is used.
- Correcting wrong matches in a review table before export (ISBN extraction picks the wrong
  ISBN when a book cites others).

### 4. Privacy and terms-of-service constraints

- Lookups send only ISBNs/DOIs plus the PC's IP: low sensitivity, though a list of ISBNs on
  liver disease still reveals an interest. Never send file names or paths.
- Open Library and Crossref ask for a contact in the User-Agent / `mailto`. Use a **project**
  contact address chosen by the owner, never her personal email.
- Open Library covers: display only, courtesy link, no bulk crawling. Rate-limit to < 100
  ISBN-keyed covers per 5 min, or resolve the cover ID through the Books API first (cover-ID
  URLs are not rate-limited **[published]**). Cache covers locally (her own private list).
- Google Books: no fee to users; keep cached data minimal and per its API terms (we could not
  load the full Google APIs ToS caching clause; check before shipping).
- Copyright: covers and page-1 renders in a private personal list are low-risk; the feature must
  not publish anything.

### 5. Cost

- API fees: $0 **[published: Open Library, Crossref and datos.bne.es charge nothing; Google Books is free within quota; see section 4]**.
- Time **[estimate]**: 300 books at Open Library's 1 req/s ≈ 5 min metadata; covers capped at
  100/5 min → 15 min for 300 by ISBN, or ~5 min via cover IDs. Run in the background with
  progress; the list is usable before covers arrive.
- Target PC **[estimate]**: ISBN/DOI scan reuses phase-3 text (no extra disk pass if done at
  extraction time; otherwise first 5 pages per PDF ≈ 0.1–0.5 s each on a busy HDD, so 300 PDFs ≈
  1–3 min). EPUB OPF parsing takes milliseconds. Page-1 rendering ≈ 0.2–1 s per PDF in the
  worker. XLSX with 300 JPEG thumbnails of ~15 KB ≈ 5 MB.

### 6. Dependencies in VFS

Phase 3 / #23 (text extraction worker: first-page text, EPUB OPF, PDF metadata, page render).
VFS-6 / #63 (bookmark URLs). Line 5 URL normaliser (DOI/ASIN/ISBN from URLs). Multi-select or
"use these results" in the phase-1 UI.

### 7. Smallest useful first step

**Offline only**: for the current result set, extract ISBN/DOI, title, author and date from EPUB
OPF and PDF metadata/first-page text, take EPUB embedded covers and page-1 renders, and export an
XLSX (floating images, works in every Excel) plus a printable HTML. No network. Then add Open
Library + Crossref enrichment behind one consent switch.

### 8. Recommendation

**Do later, right after phase 3 lands.** The local part is cheap once #23 exists, needs no AI and
no accounts, and maps directly onto her request "sin teclearlo en Excel". Ship XLSX + printable
HTML first; add online enrichment (Open Library → Google Books fallback for Spanish titles →
Crossref for DOIs) second; formatted CSL styles (hayagriva) only if she asks for a specific style.

### 9. Open product questions

1. **Is the deliverable an Excel sheet she keeps editing, a printed/PDF list, or both, and which
   Excel version does she have (365 or older)?** This decides in-cell vs floating covers.
2. Should a "work" also include bookmarked books she does *not* own (from Amazon/Casa del Libro
   links)? That would mark "lo tengo" vs "solo marcador".
3. Does she need a formal citation style (APA, Chicago), or just title/author/year/publisher?
4. Which contact email may be sent to Open Library/Crossref (project address)?

### 10. Sources

- https://www.w3.org/TR/epub-33/ (§5.5.3 metadata, §D.6.1 cover-image)
- https://www.crossref.org/blog/dois-and-matching-regular-expressions/
- https://www.crossref.org/documentation/retrieve-metadata/rest-api/access-and-authentication/
- https://crossref.org/documentation/retrieve-metadata/rest-api/rest-api-metadata-license-information
- https://openlibrary.org/dev/docs/api/covers
- https://openlibrary.org/developers/api ; https://openlibrary.org/developers/licensing
- https://developers.google.com/books/docs/v1/using ; https://developers.google.com/books/terms
- https://discuss.google.dev/t/requesting-higher-quota-for-google-books-api-bookquest-app/286093
- https://www.cultura.gob.es/webISBN/tituloSimpleFilter.do?cache=init&prev_layout=busquedaisbn&layout=busquedaisbn&language=es
- https://www.bne.es/en/catalogs/linked-data-at-the-bne
- https://fundaciongsr.org/wp-content/uploads/2021/11/9-DILVE.pdf
- https://bookscouter.com/blog/how-to-access-book-information-via-isbn-book-databases-overview/ (ISBNdb, secondary)
- https://docs.rs/rust_xlsxwriter/latest/rust_xlsxwriter/struct.Image.html
- https://docs.rs/rust_xlsxwriter/latest/rust_xlsxwriter/worksheet/struct.Worksheet.html
- https://github.com/zotero/citeproc-rs
- https://docs.rs/hayagriva/
