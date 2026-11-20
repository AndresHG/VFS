# VFS-1 — Tasks

Each task owns its files; a file has one owner at a time. The contract (public types, the
`Vfs` facade, `db.rs`, the schema, the manifests) is fixed by T1: later tasks change it only
through the lead. Paths are relative to the repository root; `core/` means
`crates/vfs-core/`.

| Task | Scope | Files owned | Depends on | AC |
|---|---|---|---|---|
| T1 | Workspace skeleton and contract: cargo workspace, contract types, error codes, folding, extension -> kind, schema v1 and migrations, `db::ingest` write helpers, index recovery, `Vfs` facade delegating to stub modules, phase-2 extension point | `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `core/Cargo.toml`, `core/src/{lib,types,error,fold,kind,db,api,content}.rs`, `core/src/schema.sql`, `crates/vfs-cli/Cargo.toml` | — | AC-15 (code part) |
| T2a | Exact search, filters, folder hits, paging, roots (add, remove, list, overlap rules) | `core/src/search.rs`, `core/src/roots.rs`, `core/tests/search.rs`, `core/tests/roots.rs` | T1 | AC-4 (add/remove rules), AC-8 (matching), AC-10, AC-11 |
| T2b | One-typo tolerance and ranking | `core/src/fuzzy.rs`, `core/src/rank.rs`, their unit tests | T1 | AC-10 |
| T3a | Indexer thread: directory listing and exclusions, diff and batched writes, resume after interruption, removal of roots, progress | `core/src/indexer/{mod,walk,apply}.rs`, `core/tests/indexer.rs` | T1 | AC-4, AC-5, AC-6, AC-8 |
| T3b | Change detection and platform code: watcher (FSEvents through `notify` on macOS, `ReadDirectoryChangesW` on Windows), volume identity, background priority | `core/src/indexer/watch.rs`, `core/src/platform.rs`, `core/tests/watch.rs` | T1 | AC-7, AC-8 |
| T4+T5 | App shell and CI: Tauri crate, configuration, icons, capabilities, commands of `spec.md` "Tauri commands and events", "mostrar en carpeta" with fallback to the parent folder, CI and release workflows | `apps/desktop/package.json`, `apps/desktop/package-lock.json`, `apps/desktop/src-tauri/**`, `.github/workflows/{ci,release}.yml`; the desktop member line in the root `Cargo.toml` | T1 | AC-3, AC-4, AC-5, AC-13, AC-14 |
| T6 | UI: one Spanish screen with search box, results as the user types, visible filters (type including folder, date, size, folder), progress, folder panel | `apps/desktop/ui/**` | the command contract only | AC-4, AC-5, AC-11, AC-12, AC-13 |
| T7 | CLI and performance harness: `vfs index`, `search`, `gen-db`, `gen-tree`, `bench`, `stats`; synthetic 2 M-file database; latency and memory report | `crates/vfs-cli/**`, `docs/perf.md` | T1; T2 for `bench`, T3 for `index` | AC-9 |
| T8 | Docs: architecture, phase-2 content search design, README | `docs/architecture.md`, `docs/content-search-design.md`, `README.md` | T1 | AC-15 |
| T9 | Integration and acceptance: app indexes and searches a real folder on the Mac, app memory with the 2 M index recorded, CI green on Windows and macOS, Windows installer downloaded | no source files; fixes go back to the owning task | T2–T8 | AC-1, AC-9, AC-14, all |

## Rules every task follows

- Tests cite the AC they prove (`/// VFS-1 AC-n: ...`).
- A stub body is `todo!("<task id>")` until its owner implements it.
- If time runs out, cut in this order: the 5 M-file bench and large `gen-tree` runs, the
  Windows volume GUID lookup, `stats`, keyboard navigation, the custom date interval,
  ranking tiers, the macOS release artifact. Never cut: the Windows installer, the live
  watcher (decision 15), the end-to-end desktop test.
