<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/brand/vfs-logo-dark.svg">
    <img src="docs/brand/vfs-logo.svg" alt="VFS logo: a folder holding a small graph" height="72">
  </picture>
</p>

<p align="center">
  <b>Find any file in a huge personal archive, by name or folder, as you type.</b><br>
  Offline desktop app for Windows and macOS · designed for 5 million files · Spanish and English interface
</p>

# VFS: offline file search for huge personal archives

VFS finds any file in a huge personal archive by its name or by the folders it sits in. It
is a desktop app for Windows 10+ and macOS, fully offline, in Spanish and English (the
system language by default, changeable in Settings).

It is a fast, effective file finder for archives of millions of files (designed for 5 million),
meant for anyone. It is tuned to stay fast and light even on a slow Windows PC whose 4–5 TB
archive sits on one spinning disk shared with a busy sync client, and runs just as well on
ordinary or fast machines. The product spec is
[`specs/vfs-1-core-search/spec.md`](specs/vfs-1-core-search/spec.md).

## What it does

- Indexes only the folders the user chooses ("carpetas"), in the background, at low CPU
  priority, pausing while the disk is busy; small updates use very-low disk priority.
  Closing the app and opening it again resumes the scan where it stopped.
- Reads names, sizes and dates from the directory listing. It never opens, moves or changes
  a user file.
- Searches as the user types. Every word must appear in the file name or in the folder path,
  ignoring case and accents ("notaria" finds "Notaría"; NFC and NFD names match). Results
  are gathered best first: names that start with the first word, names holding every word,
  then files whose folders and name together hold them, then typos.
- Tolerates one typo in words of 6 to 64 letters, when the exact search finds fewer than
  20 results. Shorter words match exactly.
- Shows folders as results too, marked as folders.
- Filters by type (documento, foto, vídeo, audio, carpeta, otro), date, size and folder.
- "Mostrar en carpeta" opens Finder or Explorer with the file or folder selected. On Windows,
  a path Explorer cannot address (over 259 characters) opens the nearest folder it can.
- Optionally includes Firefox bookmarks ("Incluir marcadores de Firefox" in the "Carpetas"
  panel, off by default; spec [VFS-6](specs/vfs-6-bookmarks/spec.md)). Bookmarks are found by
  title, address, folder path, tags and keyword with the same rules as files; bookmark
  folders are results and can be chosen as the "Carpeta" filter; "Buscar en" picks files,
  bookmarks or both, and "Orden" sorts by relevance, date or source. "Abrir en Firefox"
  opens a bookmarked page. Firefox's files are only ever copied and read: never opened in
  place, never written, and only bookmark tables are read (no history, passwords or
  cookies).
- Follows changes: a live watcher while the app runs; at each start, a replay of what
  changed while closed from the NTFS change journal (Windows) or FSEvents (macOS), or a
  catch-up re-walk (resumable, searchable meanwhile) when that history is gone; and a
  re-walk every 7 days while running. Full re-walks run one at a time; a folder that fails
  to list is retried. System, Synology and temporary files are skipped by both the walker
  and the watcher.

## What it deliberately does not do

- Search inside documents. That is planned for milestone Fase 3; the design is in
  [`docs/content-search-design.md`](docs/content-search-design.md).
- Understand photos, audio or video. They are found by name, folder and filters only.
- Open files or show previews. (A bookmark's web page is opened in Firefox on request.)
- Change anything in a source: no file, and nothing in Firefox (bookmarks are never edited,
  moved or reorganised).
- Index network drives, a NAS or removable disks. The app warns when a chosen folder is on
  one.
- Need administrator rights, read the NTFS master file table, or install a service.
- Use an LLM, OCR, code signing or auto-update (today; see where it is heading below).

## Where it is heading

The long-term aim is one all-in-one **personal knowledge base** for non-technical Windows
and Mac users, built from what a person already has: files, bookmarks, chats and notes.
Find anything across every source, ask questions over it with answers that cite the
original files (RAG), browse an Obsidian-like graph of connections generated automatically,
and keep adding notes (stored as Markdown). The data stays on the user's computer; language
models are reached through an API the user configures (Anthropic, OpenAI or another).
Sources stay read-only throughout. The direction, principles and future lines of work are
in [`docs/vision.md`](docs/vision.md).

## Repository layout

| Path | What it is |
|---|---|
| `crates/vfs-core` | Rust library: schema, folding, indexer, watcher, search. No UI dependency. |
| `crates/vfs-bookmarks` | Firefox bookmarks as a second source: profile finder, read-only reader, `bookmarks.db`, merged search. |
| `crates/vfs-cli` | `vfs` command line: `index`, `run` (the app's indexer and watcher), `search` (`--source`, `--sort`), `bookmarks`, `gen-db`, `bench`. |
| `apps/desktop/src-tauri` | Tauri 2 shell: commands, events, "mostrar en carpeta", "Abrir en Firefox". |
| `apps/desktop/ui` | The screen, in Spanish and English (`strings.es.js`, `strings.en.js`): static HTML, CSS and JavaScript, no npm build. |
| `apps/desktop/ui-dev` | A fake backend for working on the UI in a browser; never bundled. |
| `specs/` | Specs with acceptance criteria (`VFS-<n>`); tests cite them. |
| `docs/` | [Vision](docs/vision.md), [brand assets](docs/brand/README.md), [architecture](docs/architecture.md), [decisions](docs/decisions.md), [content-search design](docs/content-search-design.md). |

## Build, test and run

You need `rustup`: `rust-toolchain.toml` pins the Rust version (with `rustfmt` and
`clippy`), and rustup installs it on the first `cargo` command, and Node.js for the Tauri CLI. Tauri's own platform
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

The app window is titled "VFS — Buscador de archivos" ("VFS — File Finder" in English). Use
"Añadir carpeta…" ("Add folder…") to choose a folder; the scan starts at once and you can search while it runs.

### Command line

The CLI uses its own index file, given with `--db`:

```sh
cargo run -p vfs-cli -- index --db /tmp/vfs.db ~/Documents     # scan to completion
cargo run -p vfs-cli -- search --db /tmp/vfs.db notaria         # search
cargo run -p vfs-cli -- search --db /tmp/vfs.db --kind folder ano
cargo run -p vfs-cli -- search --help                           # all filters
cargo run -p vfs-cli -- bookmarks --db /tmp/vfs.db on           # read Firefox bookmarks
cargo run -p vfs-cli -- search --db /tmp/vfs.db --source bookmarks recetas
cargo run -p vfs-cli -- bookmarks --db /tmp/vfs.db off          # forget them again
```

`vfs bookmarks on|off|refresh|status` keeps its store in `bookmarks.db` next to the `--db`
file; `--firefox-root <dir>` points it at another folder holding `profiles.ini`.

`gen-db` writes a synthetic index shaped like a Spanish personal archive, and `bench` runs
the fixed query set of acceptance criterion AC-9 (p95 under 150 ms and under 300 MB of
memory at 2 million files) and prints PASS or FAIL:

```sh
cargo run --release -p vfs-cli -- gen-db --files 2000000 --out /tmp/vfs-2m.db
cargo run --release -p vfs-cli -- bench --db /tmp/vfs-2m.db
```

`bench` measures with warm and with cold SQLite page caches; it cannot drop the operating
system's file cache, so it does not show cold-disk latency on an HDD. The 2 M-file index is
670 MiB at schema version 4 (889 at version 2, measured, VFS-4 and VFS-7); see the status below for
the results.

## Windows installer

The installer is built by the `release` GitHub Actions workflow
([`.github/workflows/release.yml`](.github/workflows/release.yml)). It runs on Windows and
macOS runners and produces:

- `vfs-windows-installer`: an NSIS `*-setup.exe` in Spanish or English (from the system language), per-user install (no
  administrator rights), with the WebView2 bootstrapper embedded (the PC needs internet
  while installing if WebView2 is missing). It is unsigned, so Windows SmartScreen warns on
  first run and the user has to choose to run it anyway.
- `vfs-macos`: a zipped, ad-hoc signed `VFS.app`.

Each artifact carries a `SHA256SUMS.txt`; check a download with `sha256sum -c` (or
`shasum -a 256 -c` on macOS). Build provenance attestation is not published: for a private
repository GitHub offers it only on Enterprise Cloud.

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

The app keeps one SQLite database, `index.db`, in the per-user local app data folder of its
identifier `app.vfs.desktop`:

| System | Folder |
|---|---|
| macOS | `~/Library/Application Support/app.vfs.desktop/` |
| Windows | `%LOCALAPPDATA%\app.vfs.desktop\` (never the roaming folder) |

On Windows the roaming folder can be synced or backed up with the user profile, and a
multi-GB index must not travel to the NAS. An index that an earlier build left in
`%APPDATA%\app.vfs.desktop\` is moved to the local folder once, at start.

Next to it are SQLite's `index.db-wal` and `index.db-shm`, the `index.db.session/` folder
that marks an open session, `logs/vfs.log` (warnings, errors and the indexer's diagnostic
records without file names, at most about 5 MB),
and, after a recovery, `index.db.bad` until the rebuild has finished (and `index.db.new` while a recovery runs).
With Firefox bookmarks
included, `bookmarks.db` (with its `-wal` and `-shm`) holds the setting and the bookmarks
read, and `bookmarks.db.snapshot/` exists only for the moment a copy of Firefox's database
is being read. Deleting the folder while the app is closed resets VFS: the folders have to
be added again and are scanned from scratch, and the bookmark setting is off again. The indexer never indexes this folder, even when it lies inside a chosen folder.

The index has schema version 4 (version 2 added a per-file path index; versions 3 and 4 make the
index about a sixth smaller, VFS-4). An index written by an older build is upgraded in place
when the app opens it, without a rescan, and then compacted once; one written by a newer
build is refused, never overwritten.

## Status: what has and has not been run

- On macOS (the development Mac): `cargo build`, `cargo test --workspace`, `cargo fmt
  --check` and `cargo clippy` pass, and the CLI indexes and searches a real folder. The
  Windows target passes `cargo xwin clippy` (cross-compiled lint, not run).
- The app built with `tauri build` has been started on that Mac: it upgraded a version-1
  index to version 2, its catch-up re-walk at start picked up a file added and a file
  deleted while it was closed, and the watcher indexed a new folder within seconds. Its
  window was not inspected.
- AC-9 benchmark, measured on the 2 M-file synthetic index on an Apple Silicon Mac with an
  SSD, warm OS file cache: peak RSS about 123 MB at schema version 2 (138 MB at version 3
  with 16 KB pages, VFS-4). The warm p95 sits close to the 150 ms
  limit: before the no-hit probe two of three runs passed and the worst failed by under
  10 ms on two query classes; with it three of three passed, the slowest class at 114–133
  ms per run, while other work kept the load average at 4.4–5.1. Treat it as marginal.
- `cargo audit` (RustSec database of 2026-11-20, rerun after adding `lz4_flex` for VFS-6):
  no vulnerabilities. Six warnings, all in
  crates that come with Tauri: `chrono` (unmaintained; locked but not compiled),
  `proc-macro-error` and `glib` 0.18 (unmaintained and unsound, RUSTSEC-2024-0429; the Linux
  GTK stack, not built for Windows or macOS), and `zbus_names`, `zvariant`,
  `zvariant_derive` (unmaintained; the Linux D-Bus side of the single-instance plugin).
  None has a fix VFS can apply short of a Tauri update.
- **Run on Windows only on a GitHub runner** (VFS-8, Windows Server 2025, SSD; results in
  `docs/research/windows-e2e-results.md`): the NSIS installer installs and uninstalls per
  user, the window opens, the watcher follows create, rename and delete, the catch-up at start
  works, and "Mostrar en carpeta" leaves the file or folder selected in Explorer
  (`apps/desktop/src-tauri/tests/windows_shell.rs`, issue 107). Never run on Windows: for VFS-6, the copy of a `places.sqlite` that Firefox
  holds open, the Microsoft Store profile folder and "Abrir en Firefox" (`ShellExecuteW`).
- Measured on Windows only on that runner (first scan, memory, latency); nothing on an HDD or
  on the reference machine (slow 8 TB HDD). Milestone Fase 2 exists for that.

## Phases

Work is tracked as GitHub issues grouped in milestones:

| Phase | Scope |
|---|---|
| [Fase 0 · Cimientos](https://github.com/AndresHG/VFS/milestone/1) | Spec, research, engineering system, repository. |
| [Fase 1 · Búsqueda por nombre y ruta](https://github.com/AndresHG/VFS/milestone/2) | Core, indexer, watcher, app, UI, CI, Windows installer (spec VFS-1). |
| [Fase 2 · Validación en el PC real](https://github.com/AndresHG/VFS/milestone/3) | Install on the reference machine; measure first scan, cold-cache latency, memory; live alongside Synology sync. |
| [Fase 3 · Contenido de documentos](https://github.com/AndresHG/VFS/milestone/4) | Text extraction from Word/PDF and Spanish full-text search. |
| [Fase 4 · Pulido y distribución](https://github.com/AndresHG/VFS/milestone/5) | Updates, code signing, optional OCR and embeddings. |

## FAQ

**What is VFS?**
An offline desktop app for Windows 10+ and macOS that finds any file in a very large
personal archive by its name or by the names of the folders it sits in. Results appear as you
type, and "Mostrar en carpeta" opens Explorer or Finder with the file selected.

**How many files can it handle?**
It is designed for 5 million files (a 4–5 TB archive) and tuned to stay light on a slow
Windows PC with a spinning disk.

**Does it search inside documents?**
Not yet. VFS finds files by name, folder, type, date and size. Searching inside Word and PDF
files is planned for phase 3 ([design](docs/content-search-design.md)).

**Does it send my data anywhere?**
No. VFS works fully offline: no account, no cloud service, no LLM. It reads only names, sizes
and dates, and never opens, moves or changes your files. Future AI features (answers over
your archive, a knowledge graph) would be opt-in, through a model API you configure
yourself; see [`docs/vision.md`](docs/vision.md).

**Does it need administrator rights?**
No. It installs per user and does not read the NTFS master file table or install a service,
so it works the same way on Windows and macOS.

**Does it find files with typos or without accents?**
Yes. Case and accents are ignored ("notaria" finds "Notaría"), and one typo is tolerated in
longer words when the exact search finds few results.

**Can it search Firefox bookmarks?**
Yes, optionally. Turn on "Incluir marcadores de Firefox" in the "Carpetas" panel; bookmarks
are found by title, address, folder, tags and keyword, and Firefox is only ever read.

**Can it index a NAS or external disk?**
No, only internal disks. The app warns when a chosen folder is on a network or removable
drive.

**Where is it heading?**
Toward a local personal knowledge base over files, bookmarks, chats and notes: find, ask
questions with answers that cite the original files, and browse an automatically built graph
of connections. See [`docs/vision.md`](docs/vision.md).

## Contributing

Read [`CLAUDE.md`](CLAUDE.md) first: specs before code, tests cite acceptance criteria
(`/// VFS-1 AC-10: …`), branches `feat/vfs-<n>-<slug>`, squash-merged pull requests. Code,
comments and docs are in English; everything the user sees is in both string catalogs,
Spanish and English.
