# Release plan: content search (v0.3) and "ask your archive" (v0.4)

The owner decided on 2026-11-22 to ship RAG v1 in v0.4, shortly after v0.3. The specs are
[VFS-2](../specs/vfs-2-content-search/spec.md), [VFS-27](../specs/vfs-27-content-in-app/spec.md),
[VFS-28](../specs/vfs-28-ask-archive/spec.md), [VFS-34](../specs/vfs-34-file-filter/spec.md)
(file filter, added to v0.3 at the owner's request) and
[VFS-29](../specs/vfs-29-hybrid-search/spec.md) (outline). Efforts are working days of one session **[estimate]**.

## Why not both in v0.3

- **Dependency.** "Ask" retrieves from the content store. It cannot be tested end to end
  until VFS-2 and VFS-27 exist.
- **Risk.** v0.3 opens user files from the app for the first time, on a slow PC with
  Defender and Synology. One new risk per release keeps a regression attributable.
- **Pending measurement.** The document count and Defender's cost per file (VFS-27 27m) set
  the offer's estimates, and possibly a Tantivy step. They are unknown until the field
  visit.
- **Size.** v0.3 is about 18–25 days, v0.4 about 14–19. Together they would be one
  six-week release.

## v0.3: content search

| Order | PR | What | Effort |
|---|---|---|---|
| 1 | VFS-2 2a | Resume PR 98, rebase, Windows CI, cloud-only skip, run-level failures, worker lifetime, size profile | 2–3 |
| 2 | VFS-2 2b | AND by default, frequency guard, `retrieve` / `passage_text` / `unread`, metadata, content golden set | 2–3 |
| 3 | VFS-34 | File filter: excluded names and extensions, size limit for reading content | 2–3 |
| 4 | VFS-27 27a | Sidecar packaging, version handshake, stopping the worker before an update | 2–3 |
| 5 | VFS-27 27b | Background content indexer: pace, probes, pause, progress, damage, diagnostics | 5–7 |
| 6 | VFS-27 27c | Offer, Ajustes, content section, unread labels, e2e | 4–5 |
| any time | VFS-27 27m | Real-PC measurement; must land before 27c ships | 1 + visit |

**Gate before release:** CI green on both platforms; 27m measured. If the real archive has
more than ~100,000 documents, add a Tantivy PR behind `retrieve` (about 3–4 days) before
release.

## v0.4: ask your archive

| Order | PR | What | Effort |
|---|---|---|---|
| 1 | VFS-28 28a | Stable self-signed macOS signing identity | 1–2 |
| 2 | VFS-28 28b | `vfs-ai` crate: providers, keys, errors, cost, `ai.db` | 4–5 |
| 3 | VFS-28 28c | Ask flow, citations, follow-ups, RAG golden set | 4–6 |
| 4 | VFS-28 28d | Interface: answer view, cost cap, «Qué se ha enviado», bibliography | 5–6 |

28b does not depend on content search, so a second session can build it during v0.3. 28a
(the macOS signing change) is merged only after v0.3 ships, to keep one new risk per
release. With 28b done during v0.3, the gap between the releases is about 10–14 days
(28a + 28c + 28d). With one session only, it is the full 14–19.

**Gate before release:**
- the RAG golden set passes on the recorded answers;
- one manual `--live` run with each provider is recorded in 28c's PR;
- cost per question is shown.

## Later

- VFS-29 (hybrid search), after its gate on the golden set.
- OCR (#28).
- MCP connector, knowledge graph, Markdown notes: each with its own spec.
