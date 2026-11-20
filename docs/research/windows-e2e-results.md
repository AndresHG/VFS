# Windows and macOS end-to-end results (VFS-8)

> Defects 1–3 below (issues 106–108) are fixed by the v0.1.1 change (PR 113); the reveal
> selection and the splash are checked by `apps/desktop/src-tauri/tests/windows_shell.rs`. The
> tables record the v0.1.0 run as measured.

First time the real VFS app was installed, launched and driven on Windows, and first time its
window was seen on either OS. Spec: `specs/vfs-8-windows-e2e/spec.md`. Workflow:
`.github/workflows/e2e.yml` (manual + nightly). Every number below is **measured** on a GitHub
hosted runner on 2026-11-20, unless marked otherwise.

Runners: Windows = `windows-latest`, Windows Server 2025 Datacenter 10.0.26100, 2 vCPU, SSD,
screen 1024×768, WebView2 157.0.4341.46. macOS = `macos-latest`, macOS 26.7 arm64, 5 vCPU.

## What passed on real Windows

| Step | Result |
|---|---|
| Silent per-user install of the release NSIS installer (`/S`) | ✅ 1.6 s, 4.7 MB; app in `%LOCALAPPDATA%\VFS\vfs-desktop.exe`; manifest `asInvoker` (no elevation); uninstall entry in HKCU only; Start menu `VFS.lnk` |
| WebView2 | ✅ present (preinstalled on the runner, so the embedded bootstrapper was **not** exercised) |
| Release build launch + screenshot | ✅ window in 2.1 s; page painted after **50 s** on the first launch on a fresh machine (blank white window until then), **0.8 s** on the second |
| Native folder dialog "Elegir carpeta" driven by UI Automation | ✅ |
| First scan of a 100,000-file Spanish tree (accents, NFD names, 5 paths > 260 chars) | ✅ finished, 182.7 s = 547 files/s (webdriver build) |
| Search while scanning | ✅ results, but the first one took **16 s** to appear |
| Search by name, middle path segment, without accents, one typo, folder hit | ✅ target ranked 1 in every case |
| Filters: type, last 7 days, size (3 classes), folder | ✅ target ranked 1 |
| "Mostrar en carpeta" on a path > 260 chars | ✅ opens the nearest ancestor + toast "Se abrió la carpeta, pero no se pudo seleccionar el archivo." |
| "Mostrar en carpeta" on a file and on a folder | ❌ Explorer opens the right folder, **item not selected** (screenshot `win-09`) |
| Live create / rename / delete while open | ✅ followed in 2.1 / 2.0 / 2.2 s |
| Close, change the tree (50 new, 1 deleted), reopen: catch-up | ✅ new files found 7.1 s after launch, deleted one gone |
| Memory, idle after a search (100k index) | app 73 MB RSS / 61 MB private; WebView2 6 processes 214 MB RSS / 125 MB private; **total 286 MB RSS** |
| Memory peak during the first scan | 457 MB RSS total (app 106, WebView2 351) |
| Uninstall `/S` | ✅ files, shortcut and HKCU entry removed; `%LOCALAPPDATA%\app.vfs.desktop` (index) **left behind** — product decision |

The journey drives an installed NSIS build with the opt-in `webdriver` feature (embedded W3C
server). `tauri-driver` + Edge WebDriver could not attach to the release build (spec decision
1); the release build got install/verify, the launch-and-screenshot test and uninstall.

macOS (short journey, `webdriver` build of the `.app`, 20,000 files): every step passed,
including Finder opening on the file **with it selected**. First scan 1.7 s (11,553 files/s);
memory after a search 274 MB RSS (app 122 + all WebKit processes 152).

## Screenshots (docs/research/windows-e2e/)

- `win-01-first-launch-blank.png` — first launch, 7 s in: blank white window.
- `win-02-first-launch-painted.png` — the real window on Windows; footer hidden by the taskbar.
- `win-03-folder-dialog.png`, `win-04-scan-progress.png`, `win-05-search-during-scan.png`
- `win-06-search-name.png`, `win-07-search-folder.png`, `win-08-type-filter.png`
- `win-09-reveal-file.png` — Explorer on "Año 2019", file not selected.
- `win-10-reveal-long-path.png`, `win-11-catch-up.png`
- `mac-01-first-run.png` … `mac-05-reveal-file.png`

## Defects seen in the screenshots and runs

1. Window (1100×720 + title bar = 1044×759 on a 1024×768 screen) is taller than a 768-px
   screen: the status line and "Carpetas" sit behind the taskbar. Issue 106.
2. "Mostrar en carpeta" on Windows does not select the item (VFS-1 AC-13). Issue 107. The
   journey's reveal steps fail until it is fixed.
3. First launch on a fresh Windows: ~50 s of blank white window with no sign of life. Issue 108.
4. First scan on Windows stalls: bursts of ~15,000 files in ~4 s, then 20–40 s with no progress
   (547 files/s overall vs 11,553 on macOS); the first search during the scan took 16 s.
   Issue 109.
5. macOS menu bar is in English (VFS, File, Edit, View, Window, Help). Issue 110.
6. Digits use a slashed zero ("2Ø19", "Ø bytes"), which an older user may read as a letter
   (font design; owner's call). Issue 111.
7. Windows "Apps" list shows the publisher as "vfs". Issue 112.

## Probe rehearsal (docs/research/tools/vfs-probe.ps1)

`-Step all` ran as a standard user created in the job (not admin: `is_admin_token=false`).
Fixed: step `info` died on `Get-Service` (no SCM access); `semantics` now reports the folder's
mtime both as listed by its parent and as read from the folder. Found: a standard user sees no
disk performance counters (`perf.csv` empty; the admin run has rows).

NTFS change journal as a **standard user** (issue 32), C: of the runner:
- `FSCTL_QUERY_USN_JOURNAL` on `C:\`, on `\\.\C:` opened with `FILE_TRAVERSE`, and on
  `\\?\Volume{…}\` as a directory: **all succeed** (journal 32 MiB, ~40 MB history).
- Opening `\\.\C:` for read: error 5. `FSCTL_READ_USN_JOURNAL` and `FSCTL_ENUM_USN_DATA` on a
  directory handle: error 5.
- `FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`: **works** — 383,798 records covering 3.5 days read in
  0.83 s; **no file names** (`records_with_name=0`); `OpenFileById` resolved 200/200 parent
  references (0.25 ms each).
- Caveat: Windows Server 2025, not Windows 10/11 client; repeat on the reference machine. Recorded on
  issue 32.

Other probe numbers (runner SSD, not the target): 4 KiB random read p50 0.17 ms at normal and
very-low priority; `FlushFileBuffers` 4 KiB p50 3.1 ms; directory listing 6,200 dirs/s cold.
Directory mtime: create/rename/delete change it, append and grandchild do not; the parent's
listing showed a delete only after 3 s.

## Slow-disk runs

Same runner type, 2 vCPU AMD EPYC 9V45, 8 GB RAM. The tree (100,000 files) and the index sit on
a 64 GB VHDX mounted as V:. The VHDX lives on the runner's C: (NVMe SSD). Each condition
starts after a dismount and remount of V: (cold cache). "Busy" adds two loads on V:: DiskSpd
doing random 64 KiB I/O with 30 % writes, capped at about 8 MB/s; and `contention.py` doing 200
open+reads/s, 5 mtime touches/s and 2 × 2 MB written, fsynced and deleted per second. Measured
on the installed `webdriver` build; times are wall clock.

| | quiet | busy |
|---|---|---|
| Launch to usable UI (WebView2 profile on V:, cold) | 25.3 s | 18.3 s |
| First scan, 100,000 files | 91.9 s, 1,089 files/s | 136.3 s, 741 files/s (+48 %) |
| Search while scanning ("factura", app round trip) | 7–39 ms | 10–47 ms |
| 10 queries after the scan: first run, round trip | 6–28 ms | 6–33 ms |
| Same, median of 5 later runs | 2.9–5.4 ms | 2.7–7.0 ms |
| Memory idle after the scan, RSS total (app + WebView2) | 400 MB (83 + 317) | 374 MB (80 + 294) |
| Private bytes idle after the scan (app + WebView2) | 58 + 115 MB | 58 + 118 MB |
| Catch-up at start after 1,000 new files and 1 delete | found in 11.8 s, idle at 11.8 s | not measured |
| Index on disk after the scan | 45 MB + 3.5 MB WAL | — |

Not measured: in the busy run, the remount between closing and reopening the app hung (from
about 15:33 until the job's 120-minute limit). So there is no catch-up time under load, no
foreground CLI scan for comparison, and no `vfs bench` on the 2,000,000-file index, quiet or
busy. `slow-disk.ps1`'s busy remount needs fixing before the weekly run gives those numbers.

Here the search round trip stayed in milliseconds while scanning. So the 16 s first search
during the scan in the journey (defect 4; a different run, tree on D:) is not explained by the
engine alone; its cause is open in issue 109.

What the emulation can and cannot say: no tool on a hosted runner rate-limits I/O or adds seek
latency without a custom driver (Hyper-V QoS needs a VM, Storage QoS a cluster, job-object I/O
rate control is unsupported since Windows 10 1607). The runs therefore measure **contention on
an SSD-backed virtual disk with a cold cache**, not a 5,400-rpm HDD: they show how the app
behaves when other processes keep the same disk busy, never absolute times for the target.

## Only the real PC can tell

Cold-cache HDD seek costs for scan and search; first scan of 4–5 TB with Synology and Firefox
active; real Defender on a slow CPU; the WebView2 first-launch time on that machine; the USN
journal on Windows 10/11 with a standard user; screen resolution (does defect 1 hit?);
Explorer selection behaviour on that Windows build; memory with the owner's usual Firefox
load.

## Local Windows 11 ARM VM on an Apple Silicon Mac (note for the owner)

- **Parallels Desktop** (paid subscription, 14-day trial): "Get Windows 11 from Microsoft"
  installs it automatically; Microsoft lists it as an authorised way to run Windows 11 on
  M-series Macs. About an hour including the download.
- **VMware Fusion** (free): needs the Windows 11 Arm64 ISO from
  microsoft.com/software-download/windows11arm64 and a few manual settings (vTPM). 1–2 h.
- **UTM** (free, open source): same ISO, slower graphics, more manual. 1–2 h.
- Windows itself needs a licence to activate; it runs unactivated with limits.
- VFS's x64 installer runs there under Windows' x64 emulation (Prism) and the disk is the Mac's
  SSD: good for clicking through the UI, not for performance numbers.
