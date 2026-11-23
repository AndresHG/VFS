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
**personal knowledge base** that runs on your own Windows or Mac computer, as one
all-in-one product a non-technical person can use without assembling tools:

1. **Find** anything, across every source, by name, folder, content or topic.
2. **Ask** questions over it (retrieval-augmented generation, RAG): answers built from your
   own files and notes, each pointing back to the file or message it came from.
3. **Connect** it: a graph of related documents, people, places and topics, in the spirit
   of Obsidian's knowledge graph, but generated automatically by language models from the
   archive instead of linked by hand.
4. **Grow** it: add things over time (a note, a link, a list, a new source) and have them
   join the same search, answers and graph.

People already look for things this way: by topic across sources, not by file name
("everything I have about a health condition", "recipes that use two given ingredients",
"photos of one place, in chronological order"). Name and path search does not answer
those; content, concept and cross-source search do.

## Principles that hold as the scope grows

- **Your data stays local; the intelligence comes from an API.** The archive, the index and
  the user's notes stay on the user's computer. Language-model work (answers, graph
  generation, enrichment) goes through a model API the user configures with their own key
  (Anthropic, OpenAI or another provider), not a language model running on the PC
  (embeddings for search may be the one exception, see below). The API is
  opt-in and the user sees what is sent (roadmap, issue 38).
- **Read-only over sources.** VFS never moves, edits, reorganises or deletes anything in a
  source (files, bookmarks, chats). What the user adds (notes, links, tags) lives in VFS's
  own store, apart from the sources.
- **Every answer is traceable.** A result or an answer always links back to the original
  file, bookmark or message ("mostrar en carpeta").
- **One product, for non-technical users.** Everything (sources, search, answers, graph,
  notes) is inside VFS on Windows and macOS; no plugins, vaults or manual linking to set
  up. Footprint on a slow PC, background-friendliness and a simple interface come before
  features.
- **Lightweight above all.** VFS is a program centred on optimisation and lightness: it must
  never weigh on the computer it runs on, from a slow Windows PC with a spinning disk and a
  busy sync client to a fast laptop. That is why the core is Rust. Every new
  feature, source or research line is judged first by its cost in memory, CPU, disk I/O and
  index size, and background work yields to the user and to other programs by measuring
  real load. Where an open-source framework or library is too heavy or slow for this, we
  measure it and are willing to build our own focused replacement.
- **Sources are a first-class concept.** The user chooses which sources to include and can
  search one, several or all of them.

## Future lines of work

Grouped by theme, not ordered. The decided order of the lines that came from user
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
- Plain Markdown folders, and perhaps an existing Obsidian vault (depends on the research
  in issue 119).

### Content and concept search

- Text inside documents (Word, PDF): milestone Fase 3, designed in
  [`docs/content-search-design.md`](content-search-design.md).
- Semantic search with embeddings, so a query finds a concept even when the words differ:
  through the configured model API, or a small local model once issue 41 has measured one
  on the reference machine.
- Extend the existing sort by date and by source to every new source.
- A bibliographic list built from the documents found (issue 84).

### Ask your archive (RAG)

- Natural-language questions answered from the user's own content, with citations to the
  exact files and messages used.
- The retrieval layer is the same index as search; the answer is written by the model API
  the user configured (Anthropic, OpenAI or another provider), which is pluggable.
  Options, costs and the phased plan:
  [`docs/research/content-vector-rag.md`](research/content-vector-rag.md).
- A connector (MCP server) that lets the AI app the user already has (Claude Desktop, ChatGPT)
  query VFS's index, as some Zotero plugins do: a later line, after the built-in questions.

### Knowledge graph

- Links between items: same topic, same people, same place, same time, references between
  documents.
- A graph or "related items" view to browse the archive by connection, not only by query.
- Built automatically: the configured model API extracts topics, people, places and links
  in the background, so the user never links notes by hand.
- Obsidian is the inspiration for the concept, not a dependency: VFS does not require or
  wrap Obsidian. What Obsidian and its ecosystem (plugins, AI skills) already do, and
  whether VFS should interoperate with it, is a research line for later (see open
  questions).

### Adding to the knowledge base

- Capture inside VFS: a note, a link, a list, tags or a collection, searched like any
  source.
- What the user adds is stored as plain Markdown files in VFS's own folder, never in a
  source. They are knowledge-base material, written for VFS and for systems built on top
  of it (models, agents, Obsidian-like tools) to read, not documents meant to be opened
  and polished like a Word file.
- An interface for curating: grouping results into collections, annotating them, and seeing
  what was added when.

## Out of scope (still)

- Searching the open internet: a general LLM or agent already does that better.
- Changing any source, including reorganising bookmarks.
- Understanding photo, audio or video content, until the owner reopens it.

## Open questions

- Obsidian: what it and its ecosystem already offer (graph, plugins, AI skills), and
  whether VFS should read or write an Obsidian vault. Research for later, issue 119.
- How the Markdown notes are laid out (one file per note, front matter for tags and links)
  and how well Windows tools handle them.
- Chats: which apps and phones hold what the user wants (they are read only from official
  exports; see the roadmap).
