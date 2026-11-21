# Research: Windows shortcuts, linked folders, macOS aliases (for VFS)

Date: 2026-11-20. Scope: consultative, offline, no admin, never writes to sources.
Legend: **[confirmed]** = read in the cited primary source; **[empirical]** = tested on this
Mac (macOS 26 / Darwin 25.6); **[unconfirmed]** = no primary source found, flagged.

---

## 1. [MS-SHLLINK] layout needed to get a target path without COM

Spec root: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/16cb4ca1-9339-4d0c-a68d-bf1d6cc0f943
(revision 10.0, 2025-11-21). All integers little-endian.

File order: `ShellLinkHeader` → [`LinkTargetIDList`] → [`LinkInfo`] → [`StringData`...] → `ExtraData`.

### 1.1 ShellLinkHeader (fixed 0x4C = 76 bytes) [confirmed]
https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/c3376b21-0931-45e4-b2fc-a48ac0e60d15

| Off | Size | Field | Notes |
|---|---|---|---|
| 0x00 | 4 | HeaderSize | MUST be 0x0000004C |
| 0x04 | 16 | LinkCLSID | MUST be 00021401-0000-0000-C000-000000000046 (bytes on disk: `01 14 02 00 00 00 00 00 C0 00 00 00 00 00 00 46`) |
| 0x14 | 4 | LinkFlags | see 1.2 |
| 0x18 | 4 | FileAttributes | of the target, at link creation |
| 0x1C | 8 | CreationTime | FILETIME UTC of target; 0 = unset |
| 0x24 | 8 | AccessTime | |
| 0x2C | 8 | WriteTime | |
| 0x34 | 4 | FileSize | low 32 bits of target size |
| 0x38 | 4 | IconIndex | |
| 0x3C | 4 | ShowCommand | |
| 0x40 | 2 | HotKey | |
| 0x42 | 2+4+4 | Reserved1-3 | MUST be 0 |

Magic check for a cheap sniff: first 20 bytes = `4C 00 00 00` + CLSID bytes.
(The byte layout of the CLSID is the standard GUID packet representation; derived, not quoted.)

### 1.2 LinkFlags (bit A = 0x1, B = 0x2, ...) [confirmed]
https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/ae350202-3ba9-4790-9e9e-98935f4ee5af

| Bit | Mask | Name | Meaning for us |
|---|---|---|---|
| A | 0x00000001 | HasLinkTargetIDList | LinkTargetIDList follows header |
| B | 0x00000002 | HasLinkInfo | LinkInfo present |
| C | 0x00000004 | HasName | NAME_STRING (description) |
| D | 0x00000008 | HasRelativePath | RELATIVE_PATH (relative to the .lnk) |
| E | 0x00000010 | HasWorkingDir | WORKING_DIR |
| F | 0x00000020 | HasArguments | COMMAND_LINE_ARGUMENTS |
| G | 0x00000040 | HasIconLocation | ICON_LOCATION |
| H | 0x00000080 | IsUnicode | StringData is UTF-16LE; else system ANSI code page |
| I | 0x00000100 | ForceNoLinkInfo | "The LinkInfo structure is ignored" |
| J | 0x00000200 | HasExpString | EnvironmentVariableDataBlock present |
| K | 0x00000400 | RunInSeparateProcess | |
| L | 0x00000800 | Unused1 | |
| M | 0x00001000 | HasDarwinID | DarwinDataBlock (MSI) present |
| N | 0x00002000 | RunAsUser | |
| O | 0x00004000 | HasExpIcon | IconEnvironmentDataBlock |
| P | 0x00008000 | NoPidlAlias | |
| Q | 0x00010000 | Unused2 | |
| R | 0x00020000 | RunWithShimLayer | ShimDataBlock |
| S | 0x00040000 | ForceNoLinkTrack | TrackerDataBlock ignored |
| T | 0x00080000 | EnableTargetMetadata | PropertyStoreDataBlock collected |
| U | 0x00100000 | DisableLinkPathTracking | EnvironmentVariableDataBlock ignored |
| V | 0x00200000 | DisableKnownFolderTracking | Special/KnownFolder blocks ignored |
| W | 0x00400000 | DisableKnownFolderAlias | |
| X | 0x00800000 | AllowLinkToLink | |
| Y | 0x01000000 | UnaliasOnSave | |
| Z | 0x02000000 | PreferEnvironmentPath | IDList SHOULD NOT be stored; use EnvironmentVariableDataBlock path |
| AA | 0x04000000 | KeepLocalIDListForUNCTarget | |

### 1.3 LinkTargetIDList [confirmed]
https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/881d7a83-07a5-4702-93e3-f9fc34c3e1e4
`IDListSize` (u16) + IDList (`*ITEMID TERMINALID`); each ItemID is `u16 size` + data; terminal is a
u16 zero. **The ItemID payload is opaque**: "Only the creator of an item identifier knows the content
and format of the identifier. The only part of an item identifier that the Shell uses is the first two
bytes" — https://learn.microsoft.com/en-us/windows/win32/shell/links . Decoding shell items (file
entry, root folder GUID, drive, etc.) is reverse-engineered (libfwsi / winparsingtools), not in
MS-SHLLINK. A bounded parser should **skip** the IDList by its size and not decode it.

### 1.4 LinkInfo [confirmed]
https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/6813269d-0cc8-4be2-933f-e96e8e3412dc

| Off | Field |
|---|---|
| 0x00 | LinkInfoSize (u32) — "All offsets ... MUST be less than this value, and all strings ... MUST fit within" it |
| 0x04 | LinkInfoHeaderSize (u32): `0x1C` = no Unicode offsets; `>= 0x24` = Unicode offsets present |
| 0x08 | LinkInfoFlags: bit0 `VolumeIDAndLocalBasePath`, bit1 `CommonNetworkRelativeLinkAndPathSuffix` |
| 0x0C | VolumeIDOffset (0 unless bit0) |
| 0x10 | LocalBasePathOffset (0 unless bit0) |
| 0x14 | CommonNetworkRelativeLinkOffset (0 unless bit1) |
| 0x18 | CommonPathSuffixOffset (always) |
| 0x1C | LocalBasePathOffsetUnicode (only if HeaderSize >= 0x24; 0 unless bit0) |
| 0x20 | CommonPathSuffixOffsetUnicode (only if HeaderSize >= 0x24) |

All offsets are from the start of LinkInfo. `LocalBasePath` / `CommonPathSuffix` are NUL-terminated
**ANSI (system default code page)**; `*Unicode` variants are NUL-terminated UTF-16LE.
Product note <1>: "Unicode characters are stored in this structure if the data cannot be represented
as ANSI characters ... In this case, the value of the LinkInfoHeaderSize field is greater than or
equal to 36" — https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/08e6e22d-5964-4a15-abb6-3662b68d1395 .
Consequence: for most Spanish names (á, ñ representable in cp1252) Windows writes **ANSI only** in
LinkInfo, so we must decode cp1252 (or the machine's ACP) for LinkInfo paths, or prefer the
Unicode copies elsewhere (see 1.8).

**Composition** [confirmed wording]: LocalBasePath is "used to construct the full path ... by
appending the string in the CommonPathSuffix field". So:
- Local: `target = LocalBasePath(Unicode ?: ANSI) + CommonPathSuffix(Unicode ?: ANSI)`.
- Network: `target = NetName + "\" + CommonPathSuffix` (spec says NetName is e.g. `\\server\share`;
  the separator insertion is common practice, not spec text — **[unconfirmed]** exact rule).
- Usually for local targets CommonPathSuffix is empty and LocalBasePath is the full path
  (observed convention; the spec's examples section shows this, not normative).

### 1.5 VolumeID [confirmed]
https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/b7b3eea7-dbff-4275-bd58-83ba3f12d87a
`VolumeIDSize` (u32, > 0x10), `DriveType` (u32), `DriveSerialNumber` (u32), `VolumeLabelOffset`
(u32; if == 0x14 then ignore it and read `VolumeLabelOffsetUnicode` at +0x10), Data.
DriveType: 0 UNKNOWN, 1 NO_ROOT_DIR, 2 REMOVABLE, 3 FIXED, 4 REMOTE, 5 CDROM, 6 RAMDISK.
Useful for UI: "this link points to a removable drive labelled X / serial Y".

### 1.6 CommonNetworkRelativeLink [confirmed]
https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/23bb5877-e3dd-4799-9f50-79f05f938537
`Size` (>= 0x14), `Flags` (bit0 ValidDevice, bit1 ValidNetType), `NetNameOffset`, `DeviceNameOffset`
(0 unless ValidDevice), `NetworkProviderType` (WNNC_NET_*, e.g. 0x00020000 LanMan is **not** in the
spec's table — table lists 0x001A0000..0x00430000; **[unconfirmed]** why LanMan is absent),
`NetNameOffsetUnicode` / `DeviceNameOffsetUnicode` present only if `NetNameOffset > 0x14`.
NetName = "a server share path; for example, `\\server\share`". DeviceName = "a device; for example,
the drive letter `D:`" (i.e., the mapped drive letter). NetName is **always** present (not gated by a
flag); ValidNetType only gates the provider type.

### 1.7 StringData [confirmed]
https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/17b69472-0f34-4bcf-b290-eccdb8de224b
Order: NAME_STRING, RELATIVE_PATH, WORKING_DIR, COMMAND_LINE_ARGUMENTS, ICON_LOCATION, each present
iff its flag. Each = `CountCharacters` (u16, **characters**, not bytes) + string, **not**
NUL-terminated; UTF-16LE if IsUnicode else ANSI. "Unless otherwise specified, the length of each
structure MUST NOT be greater than 260"; COMMAND_LINE_ARGUMENTS is unbounded (still ≤ 65535 chars
by the u16 count).

### 1.8 ExtraData [confirmed]
https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/c41e062d-f764-4f13-bd4f-ea812ab9a4d1
Sequence of blocks `BlockSize u32, BlockSignature u32, ...`, ended by a TerminalBlock u32 `< 4`.

| Block | Signature | Size | Content |
|---|---|---|---|
| EnvironmentVariableDataBlock | 0xA0000001 | 0x314 | TargetAnsi[260] + TargetUnicode[520] — path with env vars, e.g. `%USERPROFILE%\...` ([src](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/2224d03c-f417-44a0-81e3-df169938ba72)) |
| ConsoleDataBlock | 0xA0000002 | 0xCC | console props (irrelevant) |
| TrackerDataBlock | 0xA0000003 | 0x60 | Length 0x58, Version 0, MachineID[16] NetBIOS name, Droid[32], DroidBirth[32] — used by the Link Tracking service [MS-DLTW] ([src](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/df8e3748-fba5-4524-968a-f72be06d71fc)) |
| ConsoleFEDataBlock | 0xA0000004 | 0x0C | code page |
| SpecialFolderDataBlock | 0xA0000005 | 0x10 | CSIDL integer + offset into IDList ([src](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/8833199d-fc03-4535-8011-ba36c4b3ad5f)) |
| DarwinDataBlock | 0xA0000006 | 0x314 | MSI application descriptor (Ansi "SHOULD be ignored", Unicode used) ([src](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/48f8a4c4-99fe-4787-a39f-b1367103eba8)) |
| IconEnvironmentDataBlock | 0xA0000007 | 0x314 | icon path with env vars |
| ShimDataBlock | 0xA0000008 | >= 0x88 | shim layer name |
| PropertyStoreDataBlock | 0xA0000009 | >= 0x0C | serialized property store [MS-PROPSTORE] |
| KnownFolderDataBlock | 0xA000000B | 0x1C | KNOWNFOLDERID GUID + offset into IDList ([src](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/5c7410e4-ec19-4ec5-8fff-cf4ccc46c5b6)) |
| VistaAndAboveIDListDataBlock | 0xA000000C | >= 0x0A | alternate IDList ([src](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/f020e8d5-fc9a-46d2-a536-d2267b8f37da)) |

(Signatures/sizes for Console, ConsoleFE, IconEnvironment, Shim, PropertyStore are from my reading
of the same spec family; I fetched and confirmed only the six linked pages.)

### 1.9 What different targets look like
- **Local fixed disk**: HasLinkInfo; bit0 set; VolumeID DriveType=3 + serial + label;
  LocalBasePath `C:\Users\...\file.ext`; CommonPathSuffix empty. Usually also IDList and often
  RELATIVE_PATH/WORKING_DIR. [structure confirmed; "usually" = observed convention]
- **Removable drive / offline volume**: same shape with DriveType=2 (or 5 CD-ROM). Nothing in the
  file says whether the volume is currently present; the serial + label is how Resolve finds it.
  Reading the file never needs the volume. [confirmed fields; inference on presence]
- **Network share via UNC**: bit1 set; CommonNetworkRelativeLink NetName `\\server\share`, no
  DeviceName; CommonPathSuffix = rest of path. [confirmed fields]
- **Mapped drive (Z: → \\server\share)**: bit1 set with ValidDevice; DeviceName `Z:`, NetName
  `\\server\share`; CommonPathSuffix rest. Whether bit0/VolumeID are also set for mapped drives is
  **[unconfirmed]** (implementation-dependent). We can show both `Z:\...` and `\\server\share\...`.
- **No LinkInfo** (HasLinkInfo clear or ForceNoLinkInfo set): target exists only as an IDList
  and/or EnvironmentVariableDataBlock and/or StringData. Cases:
  - Shell-namespace objects (Control Panel items, printers, "This PC", virtual folders): IDList
    only; `System.Link.TargetParsingPath` is a `::{GUID}` parsing name or VT_EMPTY "If the target
    item cannot be accessed through the Shell namespace" ([src](https://learn.microsoft.com/en-us/windows/win32/properties/props-system-link-targetparsingpath)).
    No file-system path → we should display "acceso directo a un elemento de Windows".
  - **MSI advertised shortcuts**: HasDarwinID + DarwinDataBlock 0xA0000006 with an MSI descriptor;
    the real exe path is resolved by Windows Installer at launch (SLR_INVOKE_MSI); the .lnk carries
    no reliable file path (it often has an IDList and icon pointing into `C:\Windows\Installer\{GUID}\`).
    [descriptor confirmed; "no reliable path" = **[unconfirmed]** observed behaviour]
  - **UWP / Store app shortcuts**: IDList pointing into the AppsFolder namespace
    (`shell:AppsFolder\<AUMID>`), no LinkInfo. **[unconfirmed]** — no primary source found; the
    format of those ItemIDs is undocumented.
  - Links created with `PreferEnvironmentPath`: target in EnvironmentVariableDataBlock, e.g.
    `%ProgramFiles%\App\app.exe`; expanding requires the *creating* user's environment (we can do
    best effort with the current user's env, or show it unexpanded).

---

## 2. Link resolution — what we will NOT replicate

`IShellLink::Resolve` [confirmed] (https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ishelllinkw-resolve):
1. Retrieves the stored path; if missing, uses **Distributed Link Tracking** (TrkWks client service,
   [MS-DLTW]) which "can find an object that was on an NTFS version 5.0 volume and was moved ...
   including volumes on other computers" (suppress: SLR_NOTRACK).
2. Then **search heuristics**: last known directory for same attributes + creation time, then
   "recursively searches subdirectories in the vicinity", then "the desktop and other local volumes"
   (suppress: SLR_NOSEARCH).
3. Then a dialog (suppress: SLR_NO_UI; with SLR_NO_UI the high word is a timeout, default 3000 ms).
4. SLR_NOLINKINFO disables volume-label tracking of removable media and UNC-based tracking of
   remapped drives. SLR_UPDATE may **rewrite the .lnk file** when it was loaded via IPersistFile.
The shell overview says the system resolves automatically "the next time the user selects it", and
for links in streams "the system does not automatically attempt to resolve"
(https://learn.microsoft.com/en-us/windows/win32/shell/links).

Implications for VFS: never call COM `IShellLink` (Load is harmless but Resolve can scan disks, hit
the network, and with SLR_UPDATE write to the source — violates read-only). We report the *stored*
target only, and can mark it "no encontrado" by checking **our own index**, never the disk
(this is exactly what Everything does: `exists($shortcut-target:)` checks the index —
https://www.voidtools.com/forum/viewtopic.php?t=15251, https://www.voidtools.com/forum/viewtopic.php?t=11949).

**Does reading .lnk bytes trigger resolution or network?** Resolution is an explicit API
(`Resolve`) on a COM object [confirmed above]. Reading the file with `ReadFile`/`std::fs::read` is
ordinary file I/O on the .lnk's own volume; nothing in the format is executed. I found no primary
source that states "reading bytes cannot trigger resolution" verbatim — it follows from the API
design (**inference**, high confidence). Caveat: if the *.lnk itself* lives on a network share or
cloud placeholder, reading it is network I/O / may hydrate the placeholder.

**Does stat-ing a UNC path trigger SMB?** Yes by construction: `GetFileAttributes`/`CreateFile` on
`\\server\share\...` must ask the redirector/server; for an offline server this can block for
tens of seconds. **[unconfirmed]** no Microsoft page quoting a timeout; well-known behaviour.
Rule for VFS: displaying a UNC string is pure string handling; never stat/open it during indexing
or rendering; "Mostrar en carpeta" on a network target is a user-initiated action.

---

## 3. Rust crates for .lnk parsing (crates.io API, 2026-11-20)

| Crate | Version / last release | Total dl (90d) | Licence | Deps | Notes |
|---|---|---|---|---|---|
| `lnk` (lilopkins/lnk-rs) | 0.6.4 / 2026-03-30 | 137,689 (48,873) | MIT | binrw 0.15, bitflags, chrono, encoding_rs, getset, log, num-derive/traits, substring, thiserror, uuid, winstructs (+opt clap/serde...) | Parse + write. Pure Rust, any OS. Streams with BufReader (`ShellLink::open(path, encoding)`), caller must pass the ANSI code page (encoding_rs). **Panics on spec-valid input**: `CommonNetworkRelativeLink::name()` does `panic!("either one of VALID_DEVICE or VALID_NET_TYPE must be set")` although NetName is valid without either flag; `link_target()` uses `.expect(...)`; `current_offset.rs` `.expect("invalid offset")`. Open issue #24 (2024-05) "Panic when im trying open a Powershell.lnk". Fuzz: `fuzz/` dir with one target `shell_link_header` only. 32 stars. |
| `parselnk` (rustysec) | 0.1.1 / 2022-01-28 | 54,859 (7,124) | MIT | bitflags 1, byteorder, thiserror 1, widestring 0.4 (+opt chrono) | Pure Rust. `Lnk::new(reader)` does **unbounded `read_to_end`** then parses from a Cursor. 2 `unwrap()` in hotkey read. No fuzzing found. Unmaintained since 2022 (last push 2022-08). 4 stars. |
| `lnk_parser` (AbdulRhmanAlfaifi) | 0.4.3 / 2026-02-17 | 14,537 (1,400) | MIT | byteorder, chrono, getset, serde, winparsingtools 2.1.4 (+opt clap 2, glob, serde_json) | Forensics-oriented (DFIR), decodes shell items via winparsingtools. Library code mostly `?`; unwraps are in the bin. No fuzzing found. 25 stars. Pulls serde + chrono + a large forensic dep. |
| `mslnk` | 0.1.8 / 2022-05-27 | 244,028 | MIT | — | **Writer only** ("create windows shortcuts"); not relevant. |
| `lnkr` | — | — | — | — | Unrelated (dotfile linker). |
Sources: https://crates.io/api/v1/crates/lnk , /parselnk , /lnk_parser , /mslnk ; repos
https://github.com/lilopkins/lnk-rs , https://github.com/rustysec/parselnk-rs ,
https://github.com/AbdulRhmanAlfaifi/lnk_parser ; issue https://github.com/lilopkins/lnk-rs/issues/24 .
Crate sources were downloaded and grepped (counts of unwrap/expect/panic in `src/`: lnk 40 incl.
comments/writer paths, parselnk 4, lnk_parser 28 mostly in `src/bin`). None require Windows.

**Recommendation: hand-write a bounded parser (~200–250 lines) in vfs-core.** Reasons:
1. We need ~10% of the format: header check, flags, skip IDList by size, LinkInfo paths/VolumeID/
   CNRL, StringData (relative path, working dir, maybe name), and two ExtraData blocks
   (EnvironmentVariable 0xA0000001, Darwin 0xA0000006 just to classify).
2. The most-used crate panics on valid input in exactly the path we need (network links), fuzzes
   only the header, and pulls binrw + chrono + uuid etc. into an app whose priority is footprint.
3. Bounding is trivial to guarantee by hand: read at most N bytes (e.g. 64 KiB; real links are
   typically < 4 KiB — **[unconfirmed]** typical size, no source), slice-based parsing with `get()`
   and checked arithmetic, every offset checked against LinkInfoSize/file length, every count
   against remaining bytes, `#![forbid(unsafe_code)]`, return `Option`/`Result`, never panic.
4. Add a `cargo fuzz` target (or a proptest over random/truncated/bit-flipped real samples) in CI
   to prove no panics; use `lnk` only as a dev-dependency oracle on a fixture corpus if wanted.
Decoding: UTF-16LE via `String::from_utf16_lossy`; ANSI via a tiny cp1252 table (Spanish target
machine) — `encoding_rs` is the fallback if we want all code pages.

---

## 4. `.url` Internet Shortcut files

- Microsoft documents the **object** (CLSID_InternetShortcut, IUniformResourceLocator, property set
  FMTID_Intshcut with PID_IS_URL, PID_IS_NAME, PID_IS_ICONFILE, ...) but **not the on-disk text
  layout** (https://learn.microsoft.com/en-us/windows/win32/lwef/internet-shortcuts). [confirmed absence]
- De facto format: INI text, section `[InternetShortcut]`, key `URL=...`; optional `IconFile=`,
  `IconIndex=`, `HotKey=`, `IDList=`, `Modified=`, `ShowCommand=`, `Author=`, etc. "The .URL file
  format is not officially documented"; URL limited to 2083 chars (NSIS wiki, secondary:
  https://nsis.sourceforge.io/Create_Internet_Shortcut). Media type `application/x-mswinurl`
  (https://www.ctrl.blog/entry/internet-shortcut-files/, secondary).
- Encoding: ANSI INI (Windows writes via the private-profile APIs). Non-ANSI URLs are reportedly
  stored in an extra `[InternetShortcut.W]` section (UTF-7) — **[unconfirmed]**, no primary source.
  Parser rule: read ≤ 8–16 KiB, strip UTF-8/UTF-16 BOM if present, find `[InternetShortcut]`
  case-insensitively, take first `URL=` line; ignore everything else.
- Size: typically tens to a few hundred bytes **[unconfirmed]**.
- Everything 1.5: `url:` search function "Matches internet shortcut files where the URL contains
  the specified text" (`*.url url:voidtools.com`), and `shortcut-target:` for .lnk
  (https://www.voidtools.com/support/everything/search_functions/). [confirmed]
- Explorer: hides the `.url`/`.lnk` extension and opens the URL in the default browser. Note
  `IShellLink` "cannot ... create a link to a URL" (https://learn.microsoft.com/en-us/windows/win32/shell/links).
  Relevance to VFS: consistent with existing bookmark source (VFS-6) — a `.url` is effectively a
  bookmark living in the file archive.

---

## 5. macOS: Finder aliases and symlinks

- Since 10.6, alias files contain **bookmark data**. Empirically, an alias written with
  `URL.writeBookmarkData(_:to:)` is a regular file whose data fork starts with `book\0\0\0\0mark`
  and has a `com.apple.FinderInfo` xattr with type `alis` / creator `MACS` (alias flag).
  [empirical, this Mac] The `book` binary format itself is **not documented by Apple**
  [unconfirmed]; don't parse it by hand.
- Detection: `URLResourceKey.isAliasFileKey` — **true for symlinks too** [empirical: a dangling
  symlink returned isAliasFile=true, isSymbolicLink=true]. So test `isSymbolicLink` first, or read
  the FinderInfo/`book` magic. Docs: https://developer.apple.com/documentation/foundation/urlresourcekey/isaliasfilekey
- Read target **without resolving**:
  1. `NSURL bookmarkDataWithContentsOfURL:` / `URL.bookmarkData(withContentsOf:)` — reads the alias
     file; "doesn't check to see if bookmarkFileURL points to an alias file" (I/O on the alias only)
     https://developer.apple.com/documentation/foundation/nsurl/bookmarkdata(withcontentsof:)
  2. `CFURLCreateResourcePropertiesForKeysFromBookmarkData` / `NSURL resourceValuesForKeys:
     fromBookmarkData:` with `NSURLPathKey` (also NameKey, VolumeURLKey, IsDirectoryKey).
     CF docs: "This function does not attempt to resolve the bookmark data or perform I/O."
     https://developer.apple.com/documentation/corefoundation/cfurlcreateresourcepropertiesforkeysfrombookmarkdata(_:_:_:)
  [confirmed + empirical: after deleting the target, `.pathKey` still returned the original path,
  `.volumeURLKey` = `/`]. No mount, no network.
- Resolving (what we will NOT do): `URLByResolvingAliasFileAtURL` / `URLByResolvingBookmarkData`;
  options `NSURLBookmarkResolutionWithoutMounting` ("no volume should be mounted") and
  `WithoutUI` exist precisely because resolution can mount volumes / show UI
  (https://developer.apple.com/documentation/foundation/nsurl/bookmarkresolutionoptions/withoutmounting).
- Rust access: via `objc2-foundation` / `core-foundation` FFI on macOS only (cfg(target_os="macos")).
  Network-volume aliases: the stored path is e.g. `/Volumes/Share/...` — **[unconfirmed]** whether a
  server URL key is retrievable without resolving; `NSURLVolumeURLForRemountingKey` exists but I did
  not verify it is readable from bookmark data.
- Symlinks on macOS: `lstat`/`symlink_metadata` + `read_link` is pure local I/O on the link (returns
  the stored string; may be relative). Following them has the same loop/escape issues as §6.

---

## 6. NTFS symlinks, junctions, mount points — what following implies

- **Hard links**: same volume, files only ("Hard links can't reference directories") — show up as
  separate directory entries naturally; no loop risk.
  https://learn.microsoft.com/en-us/windows/win32/fileio/hard-links-and-junctions
- **Junctions** (IO_REPARSE_TAG_MOUNT_POINT, also used for volume mount points): directories only;
  "can also link directories located on different local volumes"; cannot point to mapped network
  volumes. [same source]
- **Symbolic links** (IO_REPARSE_TAG_SYMLINK): files or dirs, absolute or relative, "can point
  directly to a remote file or directory using the UNC path"; can contain junctions/mounted folders
  in the path; relative ones are single-volume.
  https://learn.microsoft.com/en-us/windows/win32/fileio/creating-symbolic-links
  Max 63 reparse points per path (31 on XP/2003); targets may not exist.
  https://learn.microsoft.com/en-us/windows/win32/fileio/symbolic-link-programming-considerations
- Name-surrogate bit (0x20000000) marks "another named entity" — symlinks and junctions both set it.
  https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-point-tags . FindFirstFile reports
  the tag in `dwReserved0` [same source].
- **Rust std**: `FileType::new` sets `is_symlink = FILE_ATTRIBUTE_REPARSE_POINT && tag & 0x20000000`,
  so **junctions and mount points are `is_symlink()`, not `is_dir()`** with `symlink_metadata` /
  `DirEntry::file_type` (source: https://github.com/rust-lang/rust/blob/master/library/std/src/sys/fs/windows.rs ,
  `impl FileType`). Cloud placeholders (OneDrive/Synology-style IO_REPARSE_TAG_CLOUD_*) are not name
  surrogates, so they stay plain files/dirs [inference from tag bits]. walkdir's `follow_links`
  defaults off and errors on loops (https://docs.rs/walkdir/latest/walkdir/struct.WalkDir.html).
- **Risks of following**: (a) loops (classic: `C:\ProgramData\Application Data` → `C:\ProgramData`,
  cited by voidtools); (b) double counting — same files under two paths (Windows ships many
  compatibility junctions under `C:\Users\<u>`; void: following by default "would create many
  duplicates"); (c) leaving the chosen root (absolute targets anywhere); (d) crossing to another
  volume or, for symlinks, to a **network share** (SMB I/O during indexing, offline stalls);
  (e) watcher complexity: void says each USN event must be checked against every followed junction
  ("10 junctions is fine, ... 100+ would be noticeable").
  https://www.voidtools.com/forum/viewtopic.php?t=10450
- **Loop / dedup detection**: identify a directory by (volume serial, file index). "The identifier
  (low and high parts) and the volume serial number uniquely identify a file on a single computer";
  ReFS needs the 128-bit `FILE_ID_INFO` (64-bit "not guaranteed to be unique on ReFS"); FAT file IDs
  can change. https://learn.microsoft.com/en-us/windows/win32/api/fileapi/ns-fileapi-by_handle_file_information
  On macOS/Unix: (st_dev, st_ino). walkdir uses the `same-file` crate for this.
- Recommended default for VFS: index the link entry itself (name + "enlace a <target string>"), do
  not descend; optionally let the user add the target as its own root (Everything's model).

---

## 7. Prior art

**Everything (voidtools)**
- .lnk files are indexed as ordinary files by name (NTFS MFT). Everything 1.5 adds
  `shortcut-target:` and `url:` search functions [confirmed, search_functions page]. Properties
  can be added to the index to make them "instant" and are read "in the background"
  (https://www.voidtools.com/support/everything/properties/); whether Shortcut Target is indexed by
  default or read lazily is **[unconfirmed]** (the properties page does not list it). Environment
  variables in targets are expanded, `::{GUID}` targets are not (forum, developer post:
  https://www.voidtools.com/forum/viewtopic.php?t=11949). Broken-link check is index-based.
- Junctions/symlinks: "Everything does not follow junctions with NTFS/ReFS indexing"
  (https://www.voidtools.com/support/everything/indexes/). The link itself appears; its contents
  appear only under the real path. Users can add a junction manually under Options > NTFS (path +
  root), followed once (developer, 2021: https://www.voidtools.com/forum/viewtopic.php?t=10450).
  Folder indexing (used for network/FAT drives) is said by the developer to follow them, slower
  (forum, secondary to the docs). No "follow junctions" checkbox found in official docs.
- `.url`: indexed as files; `url:` function searches the URL.

**Windows Search / Explorer**
- `System.Link.TargetParsingPath`: "The Shell namespace path to the target of the link item",
  `InInvertedIndex = false`, `IsColumn = true` — i.e. stored as a column, not full-text indexed.
  https://learn.microsoft.com/en-us/windows/win32/properties/props-system-link-targetparsingpath
  Start menu search matching app shortcuts by display name is common knowledge; matching by target
  file name is **[unconfirmed]**.
- Windows Search and junctions/symlinks: only a community report that it ignores them
  (https://resource.dopus.com/t/an-idea-to-extend-windows-search-functionality/21448) — **[unconfirmed]**.

**Spotlight**: no Apple primary source found on how Spotlight treats alias files or symlinks
(aliases have content type `com.apple.alias-file`; Spotlight is not documented to index the
target) — **[unconfirmed]**.

---

## Open / unconfirmed items (summary)
1. Exact separator rule NetName + CommonPathSuffix; whether mapped-drive links also carry VolumeID.
2. UWP / AppsFolder link structure; MSI advertised links' lack of a usable path (observed only).
3. `.url` encoding (`[InternetShortcut.W]`, UTF-7) — no Microsoft doc.
4. Everything: whether Shortcut Target is indexed by default.
5. Windows Search / Spotlight handling of junctions, symlinks and aliases.
6. macOS network aliases: server URL readable from bookmark data without resolving.
