# VFS-28 — Ask your archive (RAG v1): BM25 retrieval, citations, bibliography

- **Status**: approved. The owner's decisions D1–D5 are of 2026-11-22. An independent critic
  review found 13 issues, a second round 4 more, all fixed before approval; a final
  verification pass checked them.
- **Size**: L. It sends user content to an outside service for the first time. It stores
  secrets in the OS credential store, changes the macOS signing identity, and is multi-PR.
- **Links**:
  - [VFS-2](../vfs-2-content-search/spec.md): `retrieve` (R15), chunks (D14), document
    metadata (R16).
  - [VFS-27](../vfs-27-content-in-app/spec.md): the content store in the app.
  - [VFS-29](../vfs-29-hybrid-search/spec.md): the later `Embedder`.
  - [VFS-21](../vfs-21-updates/spec.md): updates and signing.
  - [VFS-5](../vfs-5-windows-tuning/spec.md) R8: names are kept out of the log.
  - [VFS-22](../vfs-22-i18n/spec.md): string catalogs.
  - Research: `docs/research/content-vector-rag.md` §6–§7, plus three research notes of
    2026-11-22, summarised in D6–D9.
  - Issues #38, #40, #84.

## Problem

Finding a document is not the same as getting an answer from it. The owner wants to ask the
archive a question in plain Spanish («¿cuánto pagué de fianza por el piso de la calle
Mayor?») and get an answer built only from their own documents. Every claim in the answer
points to the file and page it came from, and a researcher can take away a bibliography of
the documents used. The vision keeps the data local and takes the intelligence from a model
API the user configures with their own key (`docs/vision.md`). Per question, only the
passages used may leave the PC, and the user can see them. Retrieval is BM25, already built
by VFS-2. Published work shows that an agent working over BM25 results beats plain BM25 and
a file-reading agent on large corpora (research §7). Vectors (VFS-29) improve this later but
are not a prerequisite.

## Requirements (EARS)

Settings and keys

- R1. THE Ajustes panel SHALL have a section «Preguntar a sus documentos (IA)», off by default,
  with:
  - a provider choice (Anthropic or OpenAI) and a key field;
  - «Probar», «Reemplazar» and «Borrar»;
  - the monthly cost cap (R10);
  - «Respuestas más baratas», which uses the cheap model for answers too (D7);
  - a link to «Qué se ha enviado» (R11).
  The key field is write-only. Once saved, the panel shows «Clave guardada · termina en
  …<last 4>» and never shows the key again. The help text under «Probar» says that a correct
  key can still belong to an account with no credit.
- R2. THE app SHALL keep each key in the OS credential store: Windows Credential Manager with
  local-machine persistence, or the macOS login keychain. The service is `com.vfs.app`, with
  one account per provider. After the user types it, the key SHALL never be returned to the
  web UI. It SHALL never be written to a log, a crash report (VFS-20), a settings file,
  `ai.db` or the index. Only Rust code reads it, through a secret type whose `Debug` and
  `Display` print `«redacted»`, to make a call. IF the store fails, THEN THE app SHALL
  offer to keep the key in a file only this user can read: DPAPI-encrypted on Windows, mode
  0600 on macOS, labelled «protección básica». The other choice keeps it in memory for this
  session only.
- R3. THE macOS app SHALL be signed with one stable self-signed code-signing identity, not
  ad-hoc, so that a key saved by one version is readable by the next without a prompt (D6).
  On Windows the store does not depend on the signature. The certificate SHALL be valid for at
  least 10 years, with its private key backed up offline as well as kept as a CI secret.
  IF a saved key cannot be read (certificate replaced, item damaged), THEN THE app SHALL say
  «No se puede leer la clave guardada; vuelva a introducirla» and SHALL keep working without
  AI.
- R4. WHEN the user presses «Probar», THE app SHALL validate the key with the provider's
  model-list call, which costs no tokens. It SHALL then show «Clave correcta», «La clave no
  es válida», «Sin conexión» or the provider's message.

Asking

- R5. THE search screen SHALL offer «Preguntar» only while AI is on, a key is saved and
  content search has a store (VFS-27). Without a connection, «Preguntar» SHALL say «Sin
  conexión» and leave search untouched.
- R6. WHEN the user asks their first question ever, THE app SHALL first show a one-time
  notice: the question and some passages of their documents, with file names and pages, are
  sent to <provider>, and «Qué se ha enviado» shows them. It offers «Entendido» and
  «Cancelar». WHEN the user asks a question, THE app SHALL:
  1. Send the question to the cheap model of the chosen provider, with strict structured
     output. The model returns `{keywords[], phrases[], synonyms[], kind?, date_from?,
     date_to?, folder_hint?}`.
  2. Keep `folder_hint` only if it is a folder in the name index under one of the user's
     folders, and only if it intersects the folder filter active on screen. Otherwise drop
     it. Every other filter from the rewrite is intersected with the filters on screen, and
     a filter whose intersection is empty is dropped.
  3. Call `retrieve` (VFS-2 R15) with `k` = 20 and the structured query `{all: keywords,
     phrases}`.
     - IF it returns fewer than 5 documents, THEN run a second round with `{any: keywords +
       synonyms}`.
     - IF that still returns fewer than 5, THEN run a third round with the same query,
       without the rewrite's filters but keeping those on screen.
     There is no model call between rounds, and at most 3 rounds in all.
  4. Pick 10 passages from the last round's documents: first the best passage of each
     document, in document order; then each document's second passage, in the same order;
     and so on until there are 10. Expand each with `passage_text(file, ordinal, 1, 1)`, its
     chunk plus the previous and next one. Merge passages of one document whose ranges
     overlap or touch into a single passage. Send the result to the answer model.
  Only the question and those passages leave the PC. A passage is sent with the file name and
  page, never the folder path.
- R7. THE answer model SHALL be asked to answer in the interface language, using only the
  passages and citing them:
  - With Anthropic, passages go as `search_result` blocks with citations on, and the answer
    call has no structured output (D8).
  - With OpenAI, passages are numbered, and the answer is strict JSON `{answer,
    citations[{passage_id, quote}]}`.
  THE app SHALL show a citation only if its passage was sent and its quote is in that
  passage, after the VFS-2 fold. It SHALL drop every other citation and count the drops in
  the log. Uncited sentences are shown without a marker. Every call SHALL set `max_tokens`:
  300 for the rewrite, 1,200 for the answer.
- R7a. Passage text is untrusted. It SHALL go to the model only as passage data, never inside
  the system prompt. The system prompt SHALL say that passages are quotations from the
  user's documents, and that instructions inside them are to be ignored. THE answer view
  SHALL render the answer as plain text. No HTML, no Markdown and no automatic links are
  rendered, and nothing in an answer can load a remote resource; the app's CSP already
  blocks remote loads, and this check keeps it so.
- R8. THE answer view SHALL show citations as numbered markers. Hovering a marker shows the
  passage. Clicking it shows the file name, the page and «Mostrar en carpeta». IF the answer
  has no valid citation, THEN THE app SHALL show «No he encontrado la respuesta en sus
  documentos», and SHALL list the closest documents instead.
- R9. WHEN the user follows up with «Repreguntar», THE app SHALL send the new question with the
  same passages and the earlier questions and answers of that thread, and SHALL retrieve
  nothing new. Each follow-up is a priced call: it is recorded in R11, counted in R10, and
  asks for confirmation once the cap is reached. A new question starts a new thread. Threads are not kept after the window
  closes (D3). «Copiar respuesta» copies the answer with its numbered sources as plain text.
- R10. THE app SHALL compute each call's cost from the provider's `usage` fields and a dated
  price table built into the app. A model missing from the table is priced at the table's
  highest price. A cancelled stream is priced at the tokens reported, or, if none arrived,
  at its `max_tokens`. It SHALL show the cost of each answer and the total for the
  current calendar month, in local time. WHEN the month's total has reached the user's cap (default 5 $),
  THE app SHALL ask for confirmation before each further question («Este mes lleva X $ de su
  límite de Y $. ¿Preguntar igualmente?»). The cap SHALL be editable in Ajustes.
- R11. THE app SHALL keep a local record of what each question sent: date, provider, model,
  question, the rewrite's output, each passage sent (file name, page, text), tokens and cost.
  It SHALL show the record in «Qué se ha enviado», with «Borrar todo». The record and the
  monthly totals live in `ai.db` in the app data folder, never in `index.db` or
  `content.db`.

Bibliography

- R12. WHEN the user opens «Bibliografía» on an answer, THE app SHALL list the cited documents,
  and SHALL offer «todos los encontrados» to list every retrieved document. Each entry shows
  author, year, title and path, from the document's metadata (VFS-2 R16). Missing fields
  fall back as follows:
  - no title: the file name;
  - no author: «s. a.»;
  - no year: the year of the file's modification date, marked «(fecha del archivo)».
  «Copiar» copies the list as text (`Autor (Año). Título. Ruta`). «Exportar RIS» saves a
  `.ris` file (`TY - GEN`, `AU`, `PY`, `TI`, `UR` as a `file://` URL, `N1` with the pages
  cited) that Zotero and Mendeley import.

Errors and diagnostics

- R13. THE app SHALL map provider errors to plain messages. The codes below come from the
  providers' docs, re-checked when 28b was built (2026-11-22) **[published]**. OpenAI also
  sends `credit_balance_exhausted` (no credit) and `organization_spend_limit_exceeded`,
  `project_spend_limit_exceeded` or `organization_usage_limit_exceeded` (quota) as 429s:
  - invalid or forbidden key: «La clave no es válida»;
  - billing or no credit: a 402, an Anthropic `billing_error`, or a 400 whose message reports a credit
    balance too low: «Su cuenta de <proveedor> no tiene saldo»;
  - rate limited: an Anthropic 429 with `retry-after`, or an OpenAI 429 with code
    `rate_limit_exceeded`. Also overloaded (529, 503) or another 5xx. Retry at most twice,
    honouring `retry-after` or `x-ratelimit-reset-*`, else waiting 2 s then 8 s. After that,
    show «<Proveedor> está saturado; pruebe en un momento»;
  - quota: an OpenAI `insufficient_quota`, or an error whose type or message reports a spend
    limit: «Ha llegado al límite de gasto de su cuenta». A 429 that matches no known code
    is treated as a rate limit;
  - model not found (404) or unknown model: «Este modelo ya no está disponible; actualice
    VFS»;
  - connect or DNS failure: «Sin conexión».
  Billing and quota errors SHALL never be retried. A «Cancelar» button SHALL stop a running
  question at any point.
- R14. THE diagnostic log SHALL record each question's rounds, passages sent, latency,
  tokens, cost, dropped citations and error kind. It SHALL record no question text, no
  passage text and no file or folder name.

## Acceptance scenarios

Tests cite these ids (`VFS-28 AC-<m>`) in their doc comments. Provider calls in CI go to a
local mock server that replays recorded responses. No test needs a real key.

- AC-1 (R1, R2) Given a key saved for Anthropic:
  - in `vfs-ai` (28b), the stored credential reads back. When a 401, a 500 and a forced
    panic occur, the diagnostic log, the VFS-20 report, `ai.db` and the settings file do not
    contain the key; the test greps them;
  - in the UI (28d), when Ajustes reopens, Then it shows «Clave guardada · termina en …» with
    the last 4 characters, and no IPC response carries the key. When «Borrar» is confirmed,
    Then the credential is gone and «Preguntar» disappears.
- AC-2 (R2) Given the credential store is made to fail in a test, When a key is saved, Then
  the user is offered the protected file or session-only memory. The file chosen is readable
  only by the user. On macOS the test checks mode 0600. On Windows it checks that the file's
  bytes do not contain the key and that `CryptUnprotectData` returns it.
- AC-3 (R3) Given two macOS release builds, A and B, signed with the CI identity, When A
  saves a test credential and B reads it, Then B reads it without a prompt. A CI script runs
  this check on the macOS runner without user interaction allowed, so a prompt fails the test
  instead of hanging it, and deletes the item afterwards. When the item is unreadable, Then
  the app shows the re-enter message and search still works.
- AC-4 (R4, R13) Given these mock responses, When «Probar» or a question runs, Then each case
  shows its mapped message:
  - 200;
  - 401;
  - 402;
  - an Anthropic 429 with and without `retry-after`, and an Anthropic 400 reporting a credit
    balance too low;
  - OpenAI 429s with `rate_limit_exceeded` and with `insufficient_quota`;
  - 404 for the model;
  - 529;
  - a refused connection.
  The rate-limited cases (an unmatched 429 included) and the 529 are retried at most twice.
  The 402, credit, quota and 404 cases are not retried. The exact error bodies are taken
  from the providers' error docs when 28b is built and recorded with the mock.
- AC-5 (R5, R6) Given an indexed corpus, a folder filter active on screen, and a mock
  rewrite returning a `folder_hint` outside the user's folders, When a question is asked,
  Then the hint is ignored. Every passage sent is inside the on-screen folder. When the hint
  is a real folder disjoint from the on-screen folder filter, Then it is dropped too. The
  answer request holds the question and at most 10 merged passages, with no folder path.
  When the first round returns 2 documents, Then a second round with the `any` query runs.
  When that returns 3, Then a third round without the rewrite's filters runs, and no
  fourth. Given three documents, the first with three matching passages, Then the passages
  sent are, in order: the best of documents 1, 2 and 3, then the second of document 1.
  Overlapping expansions of document 1 are merged into one passage. On
  the first question ever, the notice appears once; after «Entendido» it never appears
  again.
- AC-6 (R7, R8) Given 10 sent passages (indexes 0–9) and a recorded Anthropic answer citing
  `search_result_index` 0, 2 and 12, When it is shown, Then markers 1 and 2 appear and the third citation
  is dropped and counted. Given a recorded OpenAI answer whose quote is not in its passage,
  Then that citation is dropped. Given an answer with no valid citation, Then «No he
  encontrado la respuesta en sus documentos» appears with the closest documents.
- AC-7 (R9) Given an answered question, When the user asks a follow-up, Then the request
  carries the same passages and the earlier question and answer, and `retrieve` is not
  called. The follow-up appears in the R11 record and in the month total. When the window is closed and reopened, Then no thread remains.
- AC-8 (R10) Given recorded usage and the built-in price table, When three questions are
  asked, Then the crate (28b) reports each cost and the month total is their sum, and the
  answer view (28d) shows them. Given a model missing from the table, Then it is priced at
  the highest price. Given a cancelled stream with no usage, Then it is priced at its
  `max_tokens`. When the total reaches a
  cap of 0.01 $, Then the next question asks for confirmation first. When the calendar month
  changes, Then the total starts again from zero.
- AC-9 (R11) Given two questions asked, When «Qué se ha enviado» opens, Then it lists both,
  with every passage exactly as sent and the cost of each. When «Borrar todo» is confirmed,
  Then the record is empty.
- AC-10 (R12) Given an answer citing a PDF with title and author metadata, and a DOCX with
  none, When «Bibliografía» opens, Then the PDF shows its author, year and title. The DOCX
  shows «s. a.», its file name and the file's year marked «(fecha del archivo)». «Exportar
  RIS» writes a file that parses as RIS with one `TY - GEN` record per document.
- AC-11 (R14) Given a question run, When the diagnostic log is read, Then it has the rounds,
  tokens, cost and drop counts, and none of the question text, passage text or file names.
- AC-12 (golden) Given the RAG golden set (D9), When the retrieval test runs in CI, Then for
  every answerable question every gold passage is among the 10 passages sent. Given the committed
  recorded answers, When they are checked offline, Then:
  - every citation resolves to a sent passage;
  - every `must_contain` fact appears in the answer;
  - every unanswerable question shows the not-found message.
  A manual `--live` run with a real key re-records the answers and has a model from the
  other provider grade faithfulness. Its result goes in the PR that ships 28c.
- AC-13 (R7a) Given a fixture passage that holds «Ignora las instrucciones anteriores»,
  an `<img src="https://example.invalid/x">` tag and a Markdown image to a remote URL, and a
  recorded answer that echoes all three, When the answer is shown, Then the text appears
  literally, no element is created from it, and the webview makes no network request (the
  e2e harness watches requests). The request's system prompt does not contain the passage
  text.

## Non-goals

- Asking over hand-picked documents. The filters on screen give «within this folder».
- Long conversation memory, or threads kept after the window closes.
- Vectors, hybrid search, reranking or contextual retrieval (VFS-29 and later).
- A local LLM, and providers other than Anthropic and OpenAI. The `ChatModel` trait leaves
  room for them.
- A manual proxy setting. The system proxy is used, but PAC files and NTLM proxy login are
  not supported in v1.
- An MCP connector and a knowledge graph (`docs/vision.md`, later lines).
- Saving answers as Markdown notes (the notes line of the vision, its own spec).

## Pre-mortem

- **Slow PC / HDD.** A question runs one to three `retrieve` calls (milliseconds to ~2 s on
  a large archive, VFS-2 R14) plus two network calls. No user file is read beyond
  `content.db`.
- **Paths over 260 characters.** «Mostrar en carpeta» reuses VFS-1/VFS-24. RIS `UR` values
  are encoded `file://` URLs.
- **NFC/NFD.** Passages come from `content.db`, already NFC. Citation quotes are compared
  after the same fold.
- **Antivirus.** Nothing new is opened. `ai.db` is one more SQLite file in the app data
  folder.
- **Kill / power cut.** `ai.db` is SQLite in WAL mode. A question cut halfway leaves no
  record. The month total counts only answers whose `usage` arrived.
- **Huge result sets.** At most 10 merged passages are sent, and «todos los encontrados» lists
  the documents of the last `retrieve` call (`k` = 20).
- **Permission-denied, locked files, cloud placeholders.** n/a, because only text already in
  `content.db` is used; cloud-only files have none (VFS-2 R12).
- **User files.** Never opened or written. The only file written outside the app data folder
  is a `.ris` export, where the user chose to save it.
- **Secrets.**
  - The key never leaves Rust (R2).
  - Without the stable identity (R3), every update would break the macOS keychain item:
    measured, see D6.
  - Any process running as the same user can read a Credential Manager item. That is
    accepted, and stated in the help text.

## Phased PRs

28b does not depend on content search and can be built during v0.3. 28a is merged only after
v0.3 ships, to keep one new risk per release. 28c and 28d need VFS-2 2b and VFS-27.

| PR | Scope | Proves | Effort |
|---|---|---|---|
| 28a | macOS stable self-signed signing identity: certificate kept as a CI secret, Tauri `signingIdentity`, A/B keychain script on the macOS runner, release notes on the one-time change. | AC-3 | 1–2 days **[estimate]** |
| 28b | Crate `vfs-ai`: the `ChatModel` trait (`validate_key`, `rewrite`, streamed `answer`) for Anthropic and OpenAI over the existing `reqwest`. Also the redacting secret type, key storage with fallback, error mapping, retries, cost metering with the dated price table, `ai.db`, and the mock server for tests. | AC-1 (crate part), AC-2, AC-4, AC-8 (crate part) | 4–5 days **[estimate]** |
| 28c | Ask flow: rewrite, folder check and filter intersection, retrieval rounds, passage selection and merging, answer, citation checks, follow-ups, the untrusted-passage prompt, diagnostics. Also the RAG golden set and the recorded answers. | AC-5 (flow part), AC-6 (citation checks), AC-7, AC-11, AC-12, AC-13 (system prompt part) | 4–6 days **[estimate]** |
| 28d-1 | Ajustes and the record: the AI state and commands (settings, key save with fallback, «Probar», «Borrar», cap, record), the Ajustes section and «Qué se ha enviado». Strings go in both catalogs. | AC-1 (UI part), AC-2 (UI part), AC-4 («Probar»), AC-9 | 2–3 days **[estimate]** |
| 28d-2 | Asking: the first-use notice, «Preguntar», the plain-text answer view with markers, the cost and cap prompt, follow-ups, «Bibliografía» with copy and RIS. | AC-5 (first-use notice), AC-6 (markers and not-found view), AC-8 (UI part), AC-10, AC-13 (rendering part) | 3 days **[estimate]** |

Total ≈ 14–19 working days **[estimate]**.

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Providers in v1 | Anthropic and OpenAI, for answers. | owner | 2026-11-22 |
| D2 | Release | v0.4, shortly after v0.3, which carries content search (VFS-2, VFS-27). | owner | 2026-11-22 |
| D3 | Conversation | A single question, plus follow-ups over the same passages, with no long memory. | owner | 2026-11-22 |
| D4 | Spending | The cost of each answer is shown, plus a user-set monthly cap (default 5 $) that asks for confirmation once reached. | owner | 2026-11-22 |
| D5 | Bibliography | A copyable list and a RIS export. | owner | 2026-11-22 |
| D6 | Keys and signing | `keyring-core` with the Windows and Apple native stores. On macOS, an ad-hoc signature is a per-build hash, so after an update the new build is denied the item. A silent re-save leaves the item unreadable by both builds (measured on the development Mac with a throwaway item, 2026-11-22; Apple TN3127, TN3137). Hence the stable self-signed identity (R3) before keys ship. The data-protection keychain needs a Team ID, which the project does not have. | agent | 2026-11-22 |
| D7 | Models | Rewrite with the cheap tier: `claude-haiku-5-5`, or OpenAI's cheap tier. Answer with the mid tier: `claude-sonnet-5-5`, or OpenAI's mid tier, with thinking off or minimal. At **[estimate from published prices: 0.10/0.50 and 2/10 $ per M tokens]** that is ≈ 140 questions under the 5 $ cap. That assumes 10 passages of about 1,500 tokens each, every chunk with its neighbours (≈ 16 k tokens in, 600 out): about 0.035 $ per question. «Respuestas más baratas» uses the cheap tier for both calls, about 0.002 $ per question. Model ids and prices live in one dated table in `vfs-ai`, updated with releases. | agent | 2026-11-22 |
| D8 | Anthropic answer format | Plain text with citations on. Structured output together with citations is rejected with a 400 by the API, and a "JSON only" prompt makes citations sparse **[published, Anthropic citations docs]**. Structured output is used only for the rewrite. "Not found" is detected as an answer with no valid citation. | agent | 2026-11-22 |
| D9 | Golden set | About 50 questions (35 answerable, 15 unanswerable), in JSONL with `gold` file and page, `must_contain` and `answerable` (`crates/vfs-cli/tests/rag/`). The corpus is VFS-2's content golden corpus (`crates/vfs-content/tests/golden`, 31 documents: BOE, Spanish Wikipedia excerpts under CC BY-SA, synthetic household documents; licences in its NOTICE.md), reused rather than a second 60–80-document corpus, so one set of files carries one licence record. Every case must pass, as in VFS-11, because ±15 points of noise at this size makes a threshold meaningless. The recorded rewrites and answers are hand-written until the first `--live` run re-records them. | agent | 2026-11-23 |
| D10 | HTTP | Direct calls with the `reqwest` already in the binary (rustls, OS certificate store, system proxy), plus `eventsource-stream`. No community SDK: none is official, and they add size. | agent | 2026-11-22 |
| D11 | File names in what is sent | Each passage is sent with its file name and page, but no folder path. File names carry meaning that helps the answer. They are shown in the first-use notice and in «Qué se ha enviado». | agent (within the owner's P1, "only what is needed, visibly") | 2026-11-22 |
| D12 | Certificate continuity | A self-signed certificate valid for ≥ 10 years, kept as a CI secret with an offline backup. Losing it means one re-entry of each key (R3), never a crash. The release that switches the identity says that macOS may ask once again for folder access. | agent | 2026-11-22 |
| D13 | Flow details left open by R6 | A `folder_hint` that names no indexed folder, or several, is dropped (the filter takes one folder). The rewrite's `kind` is one of `document`, `photo`, `video`, `audio`. The third retrieval round is skipped when the rewrite added no filter, since it would repeat the second. A cancelled or failed answer is still recorded (R11) with what was sent and the cost of a cancelled stream (R10). | agent | 2026-11-23 |

## Open questions

None blocking.
