# Vision and future lines of work

This is where VFS is heading, beyond what is built today. It is the owner's direction for the
project, written down so the README, specs and new work keep it in mind. It is a living
document: replace a section when the direction changes, and add new sections as new ideas
come. Nothing here is committed scope until it has a spec (`specs/`) and an issue.

## Today

VFS is a local, offline desktop app for Windows and macOS that finds any file in a
multi-terabyte personal archive by its name and folder path, and Firefox bookmarks by
title, address and folder. It reads; it never changes a source. See the
[README](../README.md) for what is built and [`docs/decisions.md`](decisions.md) for why.

## The ambition: a personal knowledge base built from what you already have

Most people accumulate a lifetime of material in many places: folders of documents and
photos, browser bookmarks, chat apps, notes and lists. VFS aims to turn all of it into one
**personal knowledge base** that runs on your own Windows or Mac computer:

1. **Find** anything, across every source, by name, folder, content or topic.
2. **Ask** questions over it (retrieval-augmented generation, RAG): answers built from your
   own files and notes, each pointing back to the file or message it came from.
3. **Connect** it: a graph of related documents, people, places and topics, in the spirit
   of Obsidian, built from the archive instead of typed by hand.
4. **Grow** it: add things over time (a note, a link, a list, a new source) and have them
   join the same search, answers and graph.

The real end user thinks this way already. Her requests are by topic across sources, not
by file name: "everything I have about fatty liver", "recipes with grated coconut and
almond flour", "photos of the septic tank, in chronological order". Name and path search
does not answer those; content, concept and cross-source search do.

## Principles that hold as the scope grows

- **Local and private.** The archive and the index stay on the user's computer. Any model
  (embeddings, an LLM for answers) runs locally or is an explicit, optional opt-in.
- **Read-only over sources.** VFS never moves, edits, reorganises or deletes anything in a
  source (files, bookmarks, chats). What the user adds (notes, links, tags) lives in VFS's
  own store, apart from the sources.
- **Every answer is traceable.** A result or an answer always links back to the original
  file, bookmark or message ("mostrar en carpeta").
- **Built for a slow PC and a non-technical user.** Footprint, background-friendliness and a
  simple Spanish interface come before features.
- **Sources are a first-class concept.** The user chooses which sources to include and can
  search one, several or all of them.

## Future lines of work

Grouped by theme, not ordered. The decided order of the lines that came from the end user's
feedback, with costs and research, is in
[`docs/research/future-lines-roadmap.md`](research/future-lines-roadmap.md) and
[`docs/research/future-lines.md`](research/future-lines.md); this page adds the longer-term
direction around them. Each line becomes a spec when it starts.

### More sources

Each source is read-only, optional and selectable, like Firefox bookmarks (VFS-6):

- Other browsers' bookmarks (Chrome, Edge, Safari).
- Chat exports, official exports only (never the Telegram API or WhatsApp database
  decryption): WhatsApp and Telegram (messages, shared links, contacts, and shared photos
  found by name and metadata only, under the no-photo-content rule).
- Notes and lists: Google Keep and similar note or list apps, from their exports.
- Private YouTube playlists and other saved-link services.
- An existing Obsidian vault or plain Markdown folders.

### Content and concept search

- Text inside documents (Word, PDF): milestone Fase 3, designed in
  [`docs/content-search-design.md`](content-search-design.md).
- Semantic search with small local embeddings, so a query finds a concept even when the
  words differ, once issue 41 has measured them on the target PC.
- Extend the existing sort by date and by source to every new source.
- A bibliographic list built from the documents found (issue 84).

### Ask your archive (RAG)

- Natural-language questions answered from the user's own content, with citations to the
  exact files and messages used.
- The retrieval layer is the same index and embeddings as search; the model that writes the
  answer is pluggable (local first; a cloud model only if the user opts in).

### Knowledge graph

- Links between items: same topic, same people, same place, same time, references between
  documents.
- A graph or "related items" view to browse the archive by connection, not only by query.
- How this fits with Obsidian is open: export the graph and notes as an Obsidian vault,
  read a vault as one more source, or offer an Obsidian-like view inside VFS (see open
  questions).

### Adding to the knowledge base

- Capture inside VFS: a note, a link, a list, tags or a collection, stored in VFS's own data
  and searched like any source.
- An interface for curating: grouping results into collections, annotating them, and seeing
  what was added when.

## Out of scope (still)

- Searching the open internet: a general LLM or agent already does that better.
- Changing any source, including reorganising bookmarks.
- Understanding photo, audio or video content, until the owner reopens it.

## Open questions

- Obsidian: interoperate (Markdown vault in or out) or provide our own graph view? Both?
- Which local model sizes are realistic for RAG on the target PC. (Optional cloud model
  access is already defined in the roadmap: off by default, the owner's own key, a consent
  preview; issue 38.)
- Where user-added notes live, and in what format, so they stay portable (plain Markdown?).
- Chats: which apps and phones hold what the user wants (they are read only from official
  exports; see the roadmap).
