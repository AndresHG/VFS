# VFS

VFS finds any file in a huge personal archive by its name or by the folders it sits in. It
is a desktop app for Windows 10+ and macOS, fully offline, with a Spanish interface.

It is built for one non-technical person whose archive of 4–5 TB lives on a fixed internal
hard disk of a slow Windows PC, and is designed for 5 million files. The product spec is
[`specs/vfs-1-core-search/spec.md`](specs/vfs-1-core-search/spec.md).

## What it does

- Indexes only the folders the user chooses ("carpetas"), in the background, at low CPU and
  disk priority. Closing the app and opening it again resumes the scan where it stopped.
- Reads names, sizes and dates from the directory listing. It never opens, moves or changes
  a user file.
- Searches as the user types. Every word must appear in the file name or in the folder path,
  ignoring case and accents ("notaria" finds "Notaría"; NFC and NFD names match).
- Tolerates one typo in words of 6 or more letters, when the exact search finds fewer than
  20 results.
- Shows folders as results too, marked as folders.
- Filters by type (documento, foto, vídeo, audio, carpeta, otro), date, size and folder.
- "Mostrar en carpeta" opens Finder or Explorer with the file or folder selected. On Windows,
  a path Explorer cannot address (over 259 characters) opens the nearest folder it can.
- Follows changes: a live watcher while the app runs, plus a full re-walk of each folder on
  start when the last one is more than a day old, and every 7 days while running.

## What it deliberately does not do

- Search inside documents. That is a later phase; the design is in
  [`docs/content-search-design.md`](docs/content-search-design.md).
- Understand photos, audio or video. They are found by name, folder and filters only.
- Open files or show previews.
- Index network drives, a NAS or removable disks. The app warns when a chosen folder is on
  one.
- Need administrator rights, read the NTFS master file table, or install a service.
- Use an LLM, OCR, code signing or auto-update.

## Repository layout

| Path | What it is |
|---|---|
| `crates/vfs-core` | Rust library: schema, folding, indexer, watcher, search. No UI dependency. |
| `crates/vfs-cli` | `vfs` command line: `index`, `search`, `gen-db`, `bench`. |
| `apps/desktop/src-tauri` | Tauri 2 shell: commands, events, "mostrar en carpeta". |
| `apps/desktop/ui` | The Spanish screen: static HTML, CSS and JavaScript, no npm build. |
| `apps/desktop/ui-dev` | A fake backend for working on the UI in a browser; never bundled. |
| `specs/` | Specs with acceptance criteria (`VFS-<n>`); tests cite them. |
| `docs/` | [Architecture](docs/architecture.md), [decisions](docs/decisions.md), [content-search design](docs/content-search-design.md). |

## Build, test and run

You need a stable Rust toolchain (`rustup`; `rust-toolchain.toml` pins the stable channel
with `rustfmt` and `clippy`) and Node.js for the Tauri CLI. Tauri's own platform
prerequisites apply: Xcode command-line tools on macOS; on Windows, the Microsoft C++ build
tools and WebView2.

The commands are the same on both systems. Run them from the repository root unless noted.

```sh
cargo build --workspace                                   # build everything
cargo test --workspace                                    # all tests
cargo test -p vfs-core                                    # one crate
cargo fmt --all -- --check                                # format check
cargo clippy --workspace --all-targets -- -D warnings     # lint
python3 scripts/claude_md_budget.py                       # CLAUDE.md size budget
```

If `cargo` is not on your `PATH` after installing Rust, run `source "$HOME/.cargo/env"`
first (macOS).

### Desktop app

```sh
cd apps/desktop/src-tauri
npx --yes @tauri-apps/cli@2.12.6 dev                    # run with live reload
npx --yes @tauri-apps/cli@2.12.6 build --bundles app    # macOS: build VFS.app
```

The app window is titled "VFS — Buscador de archivos". Use "Añadir carpeta…" to choose a
folder; the scan starts at once and you can search while it runs.

### Command line

The CLI uses its own index file, given with `--db`:

```sh
cargo run -p vfs-cli -- index --db /tmp/vfs.db ~/Documents     # scan to completion
cargo run -p vfs-cli -- search --db /tmp/vfs.db notaria         # search
cargo run -p vfs-cli -- search --db /tmp/vfs.db --kind folder ano
cargo run -p vfs-cli -- search --help                           # all filters
```

`gen-db` writes a synthetic index shaped like a Spanish personal archive, and `bench` runs
the fixed query set of acceptance criterion AC-9 (p95 under 150 ms and under 300 MB of
memory at 2 million files) and prints PASS or FAIL:

```sh
cargo run --release -p vfs-cli -- gen-db --files 2000000 --out /tmp/vfs-2m.db
cargo run --release -p vfs-cli -- bench --db /tmp/vfs-2m.db
```

`bench` measures with warm and with cold SQLite page caches; it cannot drop the operating
system's file cache, so it does not show cold-disk latency on an HDD.

## Windows installer

The installer is built by the `release` GitHub Actions workflow
([`.github/workflows/release.yml`](.github/workflows/release.yml)). It runs on Windows and
macOS runners and produces:

- `vfs-windows-installer`: an NSIS `*-setup.exe`, Spanish, per-user install (no
  administrator rights), with the WebView2 bootstrapper embedded (the PC needs internet
  while installing if WebView2 is missing). It is unsigned, so Windows SmartScreen warns on
  first run and the user has to choose to run it anyway.
- `vfs-macos`: a zipped, ad-hoc signed `VFS.app`.

To run it, either push a tag that starts with `v`:

```sh
git tag v0.1.0
git push origin v0.1.0
```

or start it by hand with `gh workflow run release.yml --ref <branch>` (or "Run workflow" on
the Actions tab). GitHub only offers the manual start once the workflow file is on the
default branch, `main`. Then download the artifact from the run page, or with
`gh run download <run-id> -n vfs-windows-installer`.

## Where the index lives

The app keeps one SQLite database, `index.db`, in the per-user app data folder of its
identifier `app.vfs.desktop`:

| System | Folder |
|---|---|
| macOS | `~/Library/Application Support/app.vfs.desktop/` |
| Windows | `%APPDATA%\app.vfs.desktop\` (the roaming app data folder) |

Next to it are SQLite's `index.db-wal` and `index.db-shm`, the `index.db.session/` folder
that marks an open session, and, after a recovery, `index.db.bad`. Deleting the folder while
the app is closed resets VFS: the folders have to be added again and are scanned from
scratch. The indexer never indexes this folder, even when it lies inside a chosen folder.

## Status: what has and has not been run

- On macOS (the development Mac): `cargo build`, `cargo test --workspace`, `cargo fmt
  --check` and `cargo clippy` pass, and the CLI indexes and searches a real folder.
- **Nothing has been run on Windows yet.** The Windows watcher, volume identity, background
  priority, long-path reveal and the installer are compiled only on Windows and have never
  executed. The CI and release workflows have not run on GitHub yet.
- No figure has been measured on the target PC (slow 8 TB HDD). Phase 2 exists for that.

## Phases

Work is tracked as GitHub issues grouped in milestones:

| Phase | Scope |
|---|---|
| [Fase 0 · Cimientos](https://github.com/AndresHG/VFS/milestone/1) | Spec, research, engineering system, repository. |
| [Fase 1 · Búsqueda por nombre y ruta](https://github.com/AndresHG/VFS/milestone/2) | Core, indexer, watcher, app, UI, CI, Windows installer (spec VFS-1). |
| [Fase 2 · Validación en el PC real](https://github.com/AndresHG/VFS/milestone/3) | Install on the target PC; measure first scan, cold-cache latency, memory; live alongside Synology sync. |
| [Fase 3 · Contenido de documentos](https://github.com/AndresHG/VFS/milestone/4) | Text extraction from Word/PDF and Spanish full-text search. |
| [Fase 4 · Pulido y distribución](https://github.com/AndresHG/VFS/milestone/5) | Updates, code signing, optional OCR and embeddings. |

The spec calls document content search "phase 2" (after the v1 name search); on GitHub it
is milestone Fase 3, because validation on the real PC comes first.

## Contributing

Read [`CLAUDE.md`](CLAUDE.md) first: specs before code, tests cite acceptance criteria
(`/// VFS-1 AC-10: …`), branches `feat/vfs-<n>-<slug>`, squash-merged pull requests. Code,
comments and docs are in English; everything the user sees is in Spanish.
