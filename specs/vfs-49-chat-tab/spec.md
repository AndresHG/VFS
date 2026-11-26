# VFS-49 — The Chat tab: chats beside the search, kept on this PC

- **Status**: implemented (PR #235, 49a and 49b together) — owner decisions D1–D5, 2026-11-26
- **Size**: L (a new table set in `ai.db`, a retrieval change in `vfs-ai` and `vfs-content`,
  and the app's main layout; two PRs)
- **Links**: [VFS-28](../vfs-28-ask-archive/spec.md) (ask your archive, whose R5, R8 layout,
  R9 and D3 this replaces), [VFS-47](../vfs-47-model-selector/spec.md) (model selector),
  [VFS-46](../vfs-46-ask-live-tuning/spec.md) (retrieval tuning), VFS-42 (scope tabs over the
  results), [VFS-29](../vfs-29-hybrid-search/spec.md) (embeddings, later). Design canvas:
  https://claude.ai/artifact/YYW3YmDk4wn2oZyqcPFjcF

## Problem

The owner tried the 0.4.0 build and rejected how questions are asked (2026-11-26). The
«Preguntar» button sits on the top bar, a drawer opens beside the results, and «Repreguntar»
sends follow-ups over the first question's passages only. He wants asking to be a place of its
own, like the ChatGPT or Claude apps: two tabs at the top, Buscador (today's screen) and Chat;
in Chat, the chats on the left, «Nuevo chat», and a threaded conversation that always answers
from his local documents, never the web. A search result can open a chat about that one
document. Published tools that pair search with chat do the same (Raycast's root search hands
the query to its AI chat; Drive, NotebookLM and Dropbox Dash scope a chat to chosen files with
a visible, removable chip; Notion folded its separate Q&A button into one AI panel). 0.4.0 is
not tagged until this ships.

## What I know

- The ask flow is `vfs_ai::ask::{ask, follow_up}` (`crates/vfs-ai/src/ask/mod.rs`); a
  follow-up retrieves nothing (VFS-28 R9). Threads live in memory in `AiState.threads`
  (`apps/desktop/src-tauri/src/ai.rs`) and die with the window (VFS-28 D3).
- Retrieval takes `SearchFilters` (kind, dates, size, folder) through the `Corpus` trait;
  nothing selects one file. `vfs-content`'s `filter_clause` builds the SQL.
- The UI is `ask.js` driving `#ai-ask` (a drawer) and `#ask-open` in `<header class="top">`;
  the model selector `#ai-model` sits in that drawer (VFS-47). «Bibliografía», the first-use
  notice and the cap prompt are dialogs that stay.
- Grepped `apps/desktop/ui/*.js` for tabs, chat history or a per-document ask: none ship.
  VFS-42's «Nombres» / «Documentos» tabs head the results and stay inside Buscador.

## Requirements (EARS)

Layout

- R1. THE window SHALL show two tabs at the top, «Buscador» and «Chat», beside the search box
  in Buscador (so the results keep the height VFS-35 promises) and heading the conversation in
  Chat. Buscador is today's
  screen unchanged but for R13; Chat is R2–R12. Switching tabs SHALL keep each tab's state
  (query, results, scroll, open chat, unsent text), and THE app SHALL reopen on the tab it
  closed on. The «Preguntar» button, the ask drawer and «Repreguntar» SHALL be gone.
- R2. THE Chat tab SHALL show the user's chats on the left, newest activity first, grouped
  «Hoy», «Últimos 7 días» and «Anteriores», with «Nuevo chat» above them and a field that
  filters the list by title. Each chat offers «Renombrar» and «Borrar». At narrow widths the
  list folds behind a button.
- R3. A new chat SHALL show a short line on what it does (it answers only from the user's
  documents and never searches the internet) and three example questions that fill the box.
  Its title is the first question, cut at 60 characters; no model call names it.

Asking

- R4. WHEN the user sends a message in a chat, THE app SHALL run VFS-28 R6–R7 for it anew:
  the rewrite, up to three retrieval rounds and fresh passages, then the answer. The rewrite
  call SHALL also receive the chat's earlier questions, so that «¿Y me la devolvieron?»
  searches for what the chat is about; the answer call SHALL receive the earlier questions and
  answers as text and only this message's passages. Each message is priced, recorded and
  capped as a question (VFS-28 R10, R11). Supersedes VFS-28 R9.
- R5. THE composer SHALL hold the question box, the sources selector (R6), the model selector
  of VFS-47 (unchanged behaviour, now here) and a send button that becomes «Cancelar» while an
  answer streams. Enter sends; Shift+Enter is a new line.
- R6. THE sources selector SHALL offer «Todos sus documentos» (the default) and each of the
  user's folders, and, in a chat opened by R13, «Solo: <file name>». The choice belongs to the
  chat, is saved with it and can be changed between messages. The search screen's filters do
  not apply to the chat (D2).
- R7. WHILE a chat is limited to one document, retrieval SHALL search that document only. IF
  the question matches no passage of it (for example «Resúmelo»), THEN THE app SHALL send the
  document's first passages, up to the usual passage budget, instead of answering «not found».
  IF the document has no readable text, THEN the reply SHALL be the not-found answer.
- R8. THE conversation SHALL show each question on the right and each answer below it as plain
  text with numbered markers (VFS-28 R7a, R8). Under each answer: one chip per cited source
  (file name and page) that opens the passage with «Mostrar en carpeta»; then the model and the
  cost, «Copiar» and «Bibliografía». The month's spend shows under the composer. A not-found
  answer lists the closest documents as VFS-28 R8 says.
- R9. WHEN the first message of the app's life is sent, THE first-use notice of VFS-28 R6
  SHALL show first, as today.

Keeping chats

- R10. THE app SHALL keep each chat, its sources choice and every turn (question, answer
  blocks, citations, passages shown, documents found, model, cost) in `ai.db` on this PC until
  the user deletes it (D1). «Borrar» on a chat deletes it; «Borrar todos los chats» in
  Ajustes deletes them all, after a confirmation. Deleting chats does not touch «Qué se ha
  enviado» or the month's spend, and «Borrar todo» there does not delete chats. Chats are
  never sent anywhere but as R4's earlier turns of their own chat.
- R11. Opening a saved chat SHALL show it from `ai.db` without any model call; its markers,
  chips and bibliography work as when it was answered. IF a cited file has since moved or gone,
  THEN «Mostrar en carpeta» SHALL say so as search results do.

Without AI

- R12. WHILE AI is off, no key is saved or content search has no store, THE Chat tab SHALL
  still show (D3), with a page that says what the chat does, what leaves the PC, and a
  «Configurar la IA» button that opens Ajustes at the AI section; saved chats stay listed and
  readable.

From the search

- R13. WHILE a document of the name index is the selected result, THE line under the results
  that shows its location SHALL offer «Preguntar sobre este documento». It SHALL switch to the
  Chat tab with a new chat limited to that document (R6, R7) and its examples («Resúmelo»,
  «¿Qué fechas importantes tiene?», «¿Qué me obliga a hacer?»); nothing opens in the Buscador
  tab (D4, D5). Under the search box, while AI can answer, a quiet line «Preguntar en el chat:
  «<query>»» SHALL start a new chat with the query as its first message.

## Acceptance scenarios

Tests cite these ids (`VFS-49 AC-<m>`) in their doc comments.

- AC-1 (R1) Given the app on Buscador with a query and results, When the user opens Chat and
  comes back, Then the query and results are as they were; when the app restarts on Chat, it
  opens on Chat. No `#ask-open` or `#ai-ask` exists.
- AC-2 (R2, R10) Given chats created today and 10 days ago, Then the list groups them under
  «Hoy» and «Anteriores», newest first; «Renombrar» and «Borrar» change `ai.db` and the list;
  the filter field hides chats whose title does not match.
- AC-3 (R4) Given a chat whose first question was about «fianza calle Mayor», When the user
  sends «¿Y me la devolvieron?», Then the rewrite request carries the earlier question, a new
  retrieval runs, the answer request carries the earlier turn and only the new passages, and
  `ai.db` records a second priced question.
- AC-4 (R6, R7) Given a chat limited to one document, Then every `retrieve` is limited to that
  file; When nothing in it matches, Then its first passages are sent; Given a chat on «Todos
  sus documentos» with filters set on the search screen, Then retrieval uses no filter.
- AC-5 (R10, R11) Given a chat with two answered turns, When the app restarts and the chat is
  opened, Then both turns show with their markers and chips and no request reaches the mock
  server; «Bibliografía» lists the cited documents.
- AC-6 (R10) «Borrar todos los chats» empties the chat tables and leaves the record and the
  month's total; «Borrar todo» in «Qué se ha enviado» leaves the chats.
- AC-7 (R12) Given no key, Then the Chat tab shows the setup page and «Configurar la IA» opens
  Ajustes at the AI section.
- AC-8 (R13) Given a document selected in the results, When «Preguntar sobre este documento»
  is pressed, Then the Chat tab shows a new chat whose sources selector reads «Solo: <name>».
- AC-9 (R5, VFS-47 AC-4) While an answer streams the model selector is disabled and the send
  button reads «Cancelar»; pressing it stops the stream.

## Non-goals

- Embeddings (VFS-29): they join retrieval later, under R4 unchanged.
- Web search, attachments, images, voice, sharing or exporting a chat.
- A model-written chat title, or a chat search over message text.
- Limiting a chat to several hand-picked documents; one document or one folder only.
- Bookmarks as chat sources: content search does not hold their pages yet.

## Pre-mortem

- Slow PC / HDD: retrieval per message is the same BM25 call as today's first question
  (VFS-46 measured it); opening a chat reads one row set from `ai.db`. A chat of 50 turns is
  about 50 × 15 KB of JSON; the list reads titles only.
- Long paths, NFC/NFD: file names are shown as stored; scope by file id, not by path.
- Antivirus locking `ai.db`: SQLite WAL as today; a failed write shows the storage error and
  the answer stays on screen.
- Kill or power cut mid-answer: the turn is written after the answer completes, in one
  transaction with the chat's `updated_at`; a cut loses only the unfinished turn.
- Million-match queries: retrieval is capped at 20 documents and 10 passages per message.
- Permission-denied folders, symlinks, cloud placeholders: n/a, retrieval reads only the
  content store.
- User files: nothing opens, writes, moves or deletes a user file; «Abrir» asks the OS.
- Privacy: chats now persist on disk in the app data folder (D1). They hold passage text; the
  diagnostic log and report never include them (VFS-28 R14, VFS-48).
- Does it already ship? Grepped `apps/desktop/ui/*.js`: no tabs, chat list or per-document ask.

## Delivery

| PR | Scope | Proves |
|---|---|---|
| 49a | `vfs-content` retrieval limited to one file; `vfs-ai`: chat turns (R4, R7), chat tables in `ai.db` (R10); shell commands for chats beside the old ones. | AC-3, AC-4, AC-5 (store), AC-6 |
| 49b | The two tabs, the Chat tab, the sources selector, «Preguntar sobre este documento», «Borrar todos los chats»; `ask.js`, the drawer and the old commands removed. | AC-1, AC-2, AC-5 (screen), AC-7, AC-8, AC-9 |

## Decisions

| # | Question | Choice | Who | Date |
|---|----------|--------|-----|------|
| D1 | Are chats kept? | On this PC until the user deletes them. Supersedes VFS-28 D3. | owner | 2026-11-26 |
| D2 | Do the search filters apply to the chat? | No; the chat has its own sources selector. | owner | 2026-11-26 |
| D3 | Chat tab without a key | Shown, with a setup page and «Configurar la IA». | owner | 2026-11-26 |
| D4 | «Preguntar sobre este documento» | In 0.4.0, as a new chat in the Chat tab limited to that document; no panel in Buscador. | owner | 2026-11-26 |
| D5 | Layout | Tabs Buscador / Chat; chats on the left; no drawer, no «Repreguntar». | owner | 2026-11-26 |
| D6 | Follow-ups | Every message retrieves anew; the rewrite sees the earlier questions, the answer the earlier turns. | agent | 2026-11-26 |
| D7 | Search-to-chat shortcut | A clickable line under the search box, no keyboard hijack: Tab must keep moving focus. | agent | 2026-11-26 |
| D8 | Where «Preguntar sobre este documento» lives | On the selected result's location line, a real button in the tab order. Mayús+Intro already opens a link's target or a file's copies (VFS-18, VFS-19), so it is not reused. | agent | 2026-11-26 |
| D9 | The tabs' place | Beside the search box, not on a strip above it: a strip took the height of one result row and broke VFS-35's rows at small windows. | agent | 2026-11-26 |

## Open questions

None blocking.
