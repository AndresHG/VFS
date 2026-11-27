# Roadmap: what ships in each version

The version plan from 0.5 to 1.0. It turns the direction in [`vision.md`](vision.md) into
releases; each item becomes a spec (`specs/`) before it is built. Replace a row when the plan
changes; what changed goes in the PR body. Per-release gates and steps for 0.3 to 0.5 are in
[`release-plan-content-rag.md`](release-plan-content-rag.md).

## The scope rule behind every row

VFS gives access to the knowledge a person already has: it **finds** (by name, folder,
content and meaning) and it **answers** (questions and conversations with your documents,
each answer citing its source). It never changes a source. It does not grow, learn or
curate knowledge: a knowledge graph, «Temas», people and places extracted from documents,
facts learned over time, memory, notes and capture belong to the author's other project, not
to VFS (owner, 2026-11-27). A feature that would make VFS heavier waits until it can be made
light (`CLAUDE.md`, «Lightweight first»).

## Shipped

| Version | What it brought |
|---|---|
| 0.1–0.2 | Search by name and folder over millions of files, typo and accent tolerant; Firefox bookmarks (VFS-6); filters, sort, updates, Spanish and English |
| 0.3 | Search inside documents (VFS-2, VFS-27), file filter (VFS-34), «Buscar en» (VFS-38); 0.3.1 the settings window (VFS-45) |
| 0.4 | Ask your archive with your own AI key (VFS-28) in the Chat tab beside Buscador (VFS-49), model selector (VFS-47), anonymous report (VFS-48), light work after a file change (VFS-50, VFS-51) |

## Next versions

| Version | Theme | What ships | Depends on |
|---|---|---|---|
| **0.5** | Search by meaning | Hybrid search with the local e5-small model, downloaded from Ajustes and opt-in (VFS-29c, PRs 238–242). Nothing else, so the new model is the release's one new risk | Merged (PR 243); the draft release awaits the owner's test and publish |
| **0.6** | More bookmarks, related documents | Chrome and Brave bookmarks beside Firefox (VFS-44, PR 244). «Relacionados»: documents similar to a result, in Documentos and Chat, from the vectors 0.5 already stores (VFS-53, PR 246) | Both merged; the release |
| **0.7** | Chats | Telegram chats, live, opt-in and read-only through Telegram's official client API, off by default; messages searchable but never sent to the AI (VFS-52, issue 80) | The owner's `api_id` and the spike run |
| **0.8** | Your AI app reads VFS | A local, read-only MCP server so the AI app the user already has (Claude Desktop, ChatGPT) or the author's other project can search the archive and read cited passages. No writes, nothing leaves the PC unless that app sends it | Owner decision D2 |
| **0.9** | More exported sources | Firefox history (off by default, never sent to the AI), YouTube playlists from Takeout (issue 79), WhatsApp per-chat exports (issue 80); mail from an export (mbox) only if the owner chooses it (D1) | Owner decision D1 |
| **1.0** | Ready for anyone | Validation on the target PC closed (milestone Fase 2), Windows code signing (issue 27), a user guide for non-technical people (issue 128) | The field visit, a certificate |

Small items that ride in whichever release is open: content reading per folder (issue
198), sort by publication date and by source across sources (issue 82).

Not scheduled, kept as access features for later: OCR of scanned PDFs (issue 28), the
bibliographic list (issue 84), the optional embedding API (VFS-29a).

## Out of VFS

Moved to the author's other project or dropped (owner, 2026-11-27). Do not plan them here:

- Knowledge graph, «Temas», people and places (research G2–G4 of 2026-11-26).
- Notes, capture, tags, «Colecciones» with pins and annotations, and anything that grows
  the archive from inside VFS. The Obsidian research (issue 119) goes with them; a folder
  of Markdown files is already searchable as files.
- Live mail, calendar and other accounts; actions that write anywhere.
- Learned facts and memory, including Chat answers feeding a graph.

## Open owner decisions

- **D1, mail.** Recommended: read mail only from an export the user makes (mbox), in 0.9
  at the earliest; live mail stays outside VFS.
- **D2, MCP server.** Recommended: yes, read-only, in 0.8, after Chrome and Brave and
  Telegram.
