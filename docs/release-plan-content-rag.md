# Release plan: content search (v0.3), "ask your archive" (v0.4) and search by meaning (v0.5)

The owner decided on 2026-11-22 to ship RAG v1 in v0.4, shortly after v0.3. The specs are
[VFS-2](../specs/vfs-2-content-search/spec.md), [VFS-27](../specs/vfs-27-content-in-app/spec.md),
[VFS-28](../specs/vfs-28-ask-archive/spec.md), [VFS-34](../specs/vfs-34-file-filter/spec.md)
(file filter, added to v0.3 at the owner's request) and
[VFS-29](../specs/vfs-29-hybrid-search/spec.md) (search by meaning, v0.5) and
[VFS-38](../specs/vfs-38-search-modes/spec.md) («Buscar en» and the shorter filter line). Efforts are working days of one session **[estimate]**.

## Why not both in v0.3

- **Dependency.** "Ask" retrieves from the content store. It cannot be tested end to end
  until VFS-2 and VFS-27 exist.
- **Risk.** v0.3 opens user files from the app for the first time, on a slow PC with
  Defender and Synology. One new risk per release keeps a regression attributable.
- **Pending measurement.** The document count and Defender's cost per file (VFS-27 27m) set
  the offer's estimates, and possibly a Tantivy step. They are unknown until the field
  visit.
- **Size.** v0.3 is about 18–25 days (21–31 with VFS-38), v0.4 about 14–19. Together they would be one
  six-week release.

## v0.3: content search

| Order | PR | What | Effort |
|---|---|---|---|
| 1 | VFS-2 2a | Resume PR 98, rebase, Windows CI, cloud-only skip, run-level failures, worker lifetime, size profile | 2–3 |
| 2 | VFS-2 2b | AND by default, frequency guard, `retrieve` / `passage_text` / `unread`, metadata, content golden set | 2–3 |
| 3 | VFS-34 | File filter: excluded names and extensions, size limit for reading content | 2–3 |
| 4 | VFS-27 27a | Sidecar packaging, version handshake, stopping the worker before an update | 2–3 |
| 5 | VFS-27 27b | Background content indexer: pace, probes, pause, progress, damage, diagnostics | 5–7 |
| 5a | VFS-38 38a | «Buscar en» scope in the core (`scope: names`), CLI `--names-only`; independent, any time | 1–2 |
| 5b | VFS-38 38b | New filter line: scope group, «Más filtros», chips, sort in the results header, remembered choices | 2–4 |
| 6 | VFS-27 27c | Offer, Ajustes, content section, unread labels, e2e; adds the «Solo documentos» button on 38b's line | 4–5 |
| any time | VFS-27 27m | Real-PC measurement; must land before 27c ships | 1 + visit |

**Gate before release:** CI green on both platforms; 27m measured. If the real archive has
more than ~100,000 documents, add a Tantivy PR behind `retrieve` (about 3–4 days) before
release.

## v0.4: ask your archive

v0.3.0 shipped on 2026-11-25 without the AI. 28b, 28c and 28d are merged behind the desktop
Cargo feature `ai`, now on by default (`default = ["ai"]` in
`apps/desktop/src-tauri/Cargo.toml`; `default = []` turns it off, and CI keeps that build
compiling). The live runs and their fixes are VFS-46 (#218, #219, #223; Haiku 50/50, Luna
49/50 on the golden set). Cost of the switch (R11), from the release dry run 45708968171
against the v0.3.0 assets, as compressed download sizes: Windows installer 15.2 MB against
14.6 MB, macOS app (zip and updater tarball) 34.3 MB against 33.7 MB, a gap that also holds
v0.3.1's settings window. With no key the AI does no background work, makes no network call
and holds no resident memory beyond an idle `AiState`. The update check is no longer the only
connection: VFS-28 D15 supersedes VFS-21 R6, R7 and D1's wording. What is
left before v0.4:

| Order | Step | What | Who | Effort |
|---|---|---|---|---|
| 1 | Owner test | A release-candidate build (a `release` dry run) tried on the owner's Mac and, if possible, on the target PC | owner | ½ |
| 2 | Tag | Tag `v0.4.0`; replace the draft release's generated notes with hand-written ones | session; the owner publishes | ¼ |

28a, the stable macOS signing identity, shipped in v0.3.1 with the settings window (VFS-45),
so v0.4 brings one new risk (the AI) and v0.3.1 brought the other (the signature change).

**Gate before release:**
- the RAG golden set passes on recordings from a real provider, not the hand-written ones;
- one manual `--live` run with each provider is recorded in a PR;
- AC-3 passes on the macOS runner: build B reads the keychain item that build A saved;
- cost per question is shown, and the cost cap stops a question that would exceed it;
- CI and the e2e journeys are green on both platforms with the feature on;
- the footprint rule holds: with no key configured, the AI adds no background work, no
  network call and no resident memory (CLAUDE.md «Lightweight first»).

**Still open from v0.3:** the real-PC measurement 27m (busy-HDD pacing, the Windows CPU
probe, index size after compaction) needs the field visit. It does not block v0.4, but if it
shows a problem, the fix ships first as a 0.3.x.

## v0.5: search by meaning

VFS-29c shipped in four PRs (#238–#241, plus #242 for the confirmation's copy): the local model
multilingual-e5-small int8, fused with BM25, off by default, turned on in Ajustes → Documentos
after a confirmation with the download size, the time for the archive and the memory. The
`vfs-embed` worker is a release asset, never in the installer (VFS-29 D18, D21): the app
downloads it from the release of its own version, so **a v0.5 release published without the two
worker files and their `.sig` cannot turn search by meaning on**. What is left before v0.5:

| Order | Step | What | Who | Effort |
|---|---|---|---|---|
| 1 | Tag | Tag `v0.5.0`; the draft release must hold `vfs-embed-windows-x86_64.exe`, `vfs-embed-macos-aarch64` and their `.sig`, listed in `SHA256SUMS.txt` | session | ¼ |
| 2 | Owner test | The draft's build tried on the owner's Mac; the worker download works only once the release is published | owner | ½ |
| 3 | Publish | Replace the generated notes with hand-written ones and publish | owner | ¼ |

## Later

- VFS-29a (optional): the API embedder (OpenAI, Voyage), with its key and per-folder consent.
- OCR (#28).
- Which folders have their content read (#198): every indexed folder by default, with a
  «Leer contenido» switch per folder row to leave one out, in the same row as VFS-29a's
  per-folder consent (owner, 2026-11-25). Needs its own spec.
- Model selector in Ajustes (#220): choose the answer and rewrite models and the reasoning
  level, with the cost of each (owner, 2026-11-25). Needs its own spec.
- MCP connector, knowledge graph, Markdown notes: each with its own spec.
