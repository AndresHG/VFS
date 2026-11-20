<#
.SYNOPSIS
  VFS target-PC probe: measures, as a standard (non-administrator) user, every number the
  Windows design currently guesses. See docs/research/target-pc-measurement-plan.md.

.DESCRIPTION
  Read-only towards the user's files: it lists directories, watches for changes and reads the
  NTFS change journal, but never opens, modifies or deletes a user file. It writes only under
  -OutDir (default %LOCALAPPDATA%\VFS-probe) and, for the semantics, flush and seek steps, in a
  new folder vfs-probe-<timestamp> that it creates under -ScratchParent (default: the root of
  the -DataRoot drive) and deletes at the end. It refuses to run if that folder already exists,
  and never deletes anything else.

  Run from a normal PowerShell window (no "Run as administrator"):
    powershell -NoProfile -ExecutionPolicy Bypass -File vfs-probe.ps1 -Step all -DataRoot D:\

  Every step writes its results to <OutDir>\<timestamp>\*.json / *.csv and appends to log.txt.
  Compatible with Windows PowerShell 5.1 (the embedded C# keeps to C# 5).

  Status: written and syntax-checked on macOS; never executed on Windows. A failure in one
  step is logged and the next step still runs.
#>
[CmdletBinding()]
param(
  # Folder whose disk holds the archive (a root the user will add to VFS), e.g. D:\ or D:\Fotos.
  [Parameter(Mandatory = $true)] [string] $DataRoot,
  # Which step to run: info, walk, usn, usn-query, semantics, flush, seek, watch, perf, all.
  [ValidateSet('info', 'walk', 'usn', 'usn-query', 'semantics', 'flush', 'seek', 'watch', 'perf', 'all')]
  [string] $Step = 'all',
  # Minutes for the watch and perf sampling steps.
  [int] $Minutes = 15,
  # Directory cap for one walk (the walk also stops after -WalkMinutes).
  [int] $MaxDirs = 400000,
  [int] $WalkMinutes = 30,
  [string] $OutDir = (Join-Path $env:LOCALAPPDATA 'VFS-probe'),
  # Existing folder on the data disk under which a fresh scratch folder is created.
  [string] $ScratchParent = ''
)

$ErrorActionPreference = 'Continue'
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$run = Join-Path $OutDir $stamp
New-Item -ItemType Directory -Force -Path $run | Out-Null
$log = Join-Path $run 'log.txt'
if (-not $ScratchParent) { $ScratchParent = [IO.Path]::GetPathRoot($DataRoot) }
$ScratchDir = Join-Path $ScratchParent "vfs-probe-$stamp"
if (Test-Path -LiteralPath $ScratchDir) { throw "scratch folder $ScratchDir already exists; not touching it" }
$scratchCreated = $false
function New-Scratch {
  if (-not $script:scratchCreated) {
    New-Item -ItemType Directory -Path $ScratchDir | Out-Null
    $script:scratchCreated = $true
  }
  return $ScratchDir
}

function Log([string] $msg) {
  $line = '{0:HH:mm:ss} {1}' -f (Get-Date), $msg
  Write-Host $line
  Add-Content -Path $log -Value $line -Encoding UTF8
}
function Save([string] $name, $obj) {
  $obj | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $run "$name.json") -Encoding UTF8
  Log "saved $name.json"
}
function Try-Step([string] $name, [scriptblock] $body) {
  Log "=== step $name"
  try { & $body } catch { Log "step $name FAILED: $($_.Exception.Message)" }
}

$cs = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Microsoft.Win32.SafeHandles;

public static class VfsProbe
{
    // ---------- Win32 ----------
    const uint GENERIC_READ = 0x80000000, GENERIC_WRITE = 0x40000000;
    const uint FILE_LIST_DIRECTORY = 0x1, FILE_READ_ATTRIBUTES = 0x80;
    const uint SHARE_ALL = 0x7, OPEN_EXISTING = 3, CREATE_ALWAYS = 2;
    const uint FLAG_BACKUP_SEMANTICS = 0x02000000, FLAG_NO_BUFFERING = 0x20000000;
    const uint FLAG_WRITE_THROUGH = 0x80000000;
    const int FindExInfoBasic = 1, FindExSearchNameMatch = 0, FIND_FIRST_EX_LARGE_FETCH = 2;
    const int FileIdBothDirectoryInfo = 10, FileIoPriorityHintInfo = 12;
    const uint ATTR_DIRECTORY = 0x10, ATTR_REPARSE = 0x400, ATTR_OFFLINE = 0x1000;
    const uint ATTR_RECALL_ON_OPEN = 0x40000, ATTR_RECALL_ON_DATA_ACCESS = 0x400000;
    const int THREAD_MODE_BACKGROUND_BEGIN = 0x10000, THREAD_MODE_BACKGROUND_END = 0x20000;
    const uint FSCTL_QUERY_USN_JOURNAL = 0x000900f4, FSCTL_READ_USN_JOURNAL = 0x000900bb;
    const uint FSCTL_READ_UNPRIVILEGED_USN_JOURNAL = 0x000903ab, FSCTL_ENUM_USN_DATA = 0x000900b3;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct WIN32_FIND_DATAW
    {
        // FILETIMEs as two DWORDs each: a C# long would be 8-aligned and shift every field.
        public uint dwFileAttributes;
        public uint ftCreationLow, ftCreationHigh, ftLastAccessLow, ftLastAccessHigh;
        public uint ftLastWriteLow, ftLastWriteHigh;
        public uint nFileSizeHigh, nFileSizeLow, dwReserved0, dwReserved1;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string cFileName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 14)] public string cAlternateFileName;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr FindFirstFileExW(string name, int level, out WIN32_FIND_DATAW data,
        int search, IntPtr filter, int flags);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool FindNextFileW(IntPtr h, out WIN32_FIND_DATAW data);
    [DllImport("kernel32.dll")] static extern bool FindClose(IntPtr h);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr sa,
        uint disp, uint flags, IntPtr tmpl);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool GetFileInformationByHandleEx(SafeFileHandle h, int cls, byte[] buf, uint size);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetFileInformationByHandle(SafeFileHandle h, int cls, ref int info, uint size);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool DeviceIoControl(SafeFileHandle h, uint code, byte[] inBuf, int inSize,
        byte[] outBuf, int outSize, out int returned, IntPtr ov);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentThread();
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetThreadPriority(IntPtr h, int p);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool FlushFileBuffers(SafeFileHandle h);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern SafeFileHandle OpenFileById(SafeFileHandle hint, byte[] id, uint access,
        uint share, IntPtr sa, uint flags);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern uint GetFinalPathNameByHandleW(SafeFileHandle h, StringBuilder buf, uint len, uint flags);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool GetVolumeNameForVolumeMountPointW(string mount, StringBuilder name, uint len);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern IntPtr VirtualAlloc(IntPtr addr, UIntPtr size, uint type, uint protect);
    [DllImport("kernel32.dll")] static extern bool VirtualFree(IntPtr addr, UIntPtr size, uint type);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool ReadFile(SafeFileHandle h, IntPtr buf, int n, out int read, IntPtr ov);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetFilePointerEx(SafeFileHandle h, long dist, out long newPos, uint method);

    static string Long(string p)
    {
        if (p.Length == 2 && p[1] == ':') p += "\\"; // "D:" means the volume device; use "D:\\"
        if (p.StartsWith(@"\\?\")) return p;
        if (p.StartsWith(@"\\")) return @"\\?\UNC\" + p.Substring(2);
        return @"\\?\" + p;
    }

    // ---------- Step: walk ----------
    // mode 0: FindFirstFileExW + FindExInfoBasic (what Rust's std::fs::read_dir does today)
    // mode 1: same + FIND_FIRST_EX_LARGE_FETCH
    // mode 2: GetFileInformationByHandleEx(FileIdBothDirectoryInfo), 64 KB buffer
    // alternateEvery > 0: toggles background mode every that many directories, so a single
    // cold walk compares background and normal priority under the same disk conditions.
    public static Dictionary<string, object> Walk(string root, int mode, bool background,
        int maxDirs, int maxSeconds, int alternateEvery)
    {
        var r = new Dictionary<string, object>();
        bool inBg = false;
        if (background)
        {
            if (SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN)) inBg = true;
            else r["background_error"] = Marshal.GetLastWin32Error();
        }
        long bgDirs = 0, fgDirs = 0; double bgMs = 0, fgMs = 0;
        long dirs = 0, files = 0, reparse = 0, placeholders = 0, longPaths = 0, errors = 0;
        int maxDepth = 0;
        var perDir = new List<double>();
        var sw = Stopwatch.StartNew();
        var stack = new Stack<KeyValuePair<string, int>>();
        stack.Push(new KeyValuePair<string, int>(root.TrimEnd('\\'), 0));
        byte[] buf = new byte[64 * 1024];
        while (stack.Count > 0 && dirs < maxDirs && sw.Elapsed.TotalSeconds < maxSeconds)
        {
            var cur = stack.Pop();
            if (alternateEvery > 0 && dirs > 0 && dirs % alternateEvery == 0)
            {
                inBg = inBg ? !SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END)
                            : SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
            }
            double t0 = sw.Elapsed.TotalMilliseconds;
            dirs++;
            if (cur.Value > maxDepth) maxDepth = cur.Value;
            var children = new List<string>();
            var names = new List<KeyValuePair<string, uint>>();
            bool ok = mode == 2 ? ListByHandle(cur.Key, buf, names) : ListFind(cur.Key, mode == 1, names);
            if (!ok) { errors++; continue; }
            foreach (var n in names)
            {
                string full = cur.Key + "\\" + n.Key;
                if (full.Length >= 260) longPaths++;
                uint a = n.Value;
                if ((a & (ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS | ATTR_OFFLINE)) != 0) placeholders++;
                if ((a & ATTR_DIRECTORY) != 0)
                {
                    if ((a & ATTR_REPARSE) != 0) { reparse++; continue; }
                    children.Add(full);
                }
                else files++;
            }
            foreach (var c in children) stack.Push(new KeyValuePair<string, int>(c, cur.Value + 1));
            double spent = sw.Elapsed.TotalMilliseconds - t0;
            perDir.Add(spent);
            if (inBg) { bgDirs++; bgMs += spent; } else { fgDirs++; fgMs += spent; }
        }
        sw.Stop();
        if (inBg) SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END);
        r["background_dirs"] = bgDirs; r["normal_dirs"] = fgDirs;
        if (bgMs > 0) r["background_dirs_per_sec"] = bgDirs / (bgMs / 1000);
        if (fgMs > 0) r["normal_dirs_per_sec"] = fgDirs / (fgMs / 1000);
        perDir.Sort();
        r["mode"] = mode; r["background"] = background;
        r["dirs"] = dirs; r["files"] = files; r["reparse_dirs_skipped"] = reparse;
        r["placeholders"] = placeholders; r["paths_over_259"] = longPaths; r["errors"] = errors;
        r["max_depth"] = maxDepth; r["seconds"] = sw.Elapsed.TotalSeconds;
        r["finished"] = stack.Count == 0;
        r["dirs_per_sec"] = dirs / Math.Max(sw.Elapsed.TotalSeconds, 0.001);
        r["entries_per_sec"] = (dirs + files) / Math.Max(sw.Elapsed.TotalSeconds, 0.001);
        if (perDir.Count > 0)
        {
            r["ms_per_dir_p50"] = perDir[perDir.Count / 2];
            r["ms_per_dir_p95"] = perDir[(int)(perDir.Count * 0.95)];
            r["ms_per_dir_max"] = perDir[perDir.Count - 1];
        }
        return r;
    }

    static bool ListFind(string dir, bool large, List<KeyValuePair<string, uint>> outNames)
    {
        WIN32_FIND_DATAW d;
        IntPtr h = FindFirstFileExW(Long(dir).TrimEnd('\\') + "\\*", FindExInfoBasic, out d, FindExSearchNameMatch,
            IntPtr.Zero, large ? FIND_FIRST_EX_LARGE_FETCH : 0);
        if (h == new IntPtr(-1)) return false;
        do
        {
            if (d.cFileName == "." || d.cFileName == "..") continue;
            outNames.Add(new KeyValuePair<string, uint>(d.cFileName, d.dwFileAttributes));
        } while (FindNextFileW(h, out d));
        FindClose(h);
        return true;
    }

    // FILE_ID_BOTH_DIR_INFO: NextEntryOffset @0, FileAttributes @56, FileNameLength @60,
    // FileId @96, FileName @104.
    static bool ListByHandle(string dir, byte[] buf, List<KeyValuePair<string, uint>> outNames)
    {
        using (var h = CreateFileW(Long(dir), FILE_LIST_DIRECTORY, SHARE_ALL, IntPtr.Zero,
            OPEN_EXISTING, FLAG_BACKUP_SEMANTICS, IntPtr.Zero))
        {
            if (h.IsInvalid) return false;
            while (GetFileInformationByHandleEx(h, FileIdBothDirectoryInfo, buf, (uint)buf.Length))
            {
                int off = 0;
                while (true)
                {
                    int next = BitConverter.ToInt32(buf, off);
                    uint attr = BitConverter.ToUInt32(buf, off + 56);
                    int len = BitConverter.ToInt32(buf, off + 60);
                    string name = Encoding.Unicode.GetString(buf, off + 104, len);
                    if (name != "." && name != "..")
                        outNames.Add(new KeyValuePair<string, uint>(name, attr));
                    if (next == 0) break;
                    off += next;
                }
            }
            return Marshal.GetLastWin32Error() == 18; // ERROR_NO_MORE_FILES
        }
    }

    // ---------- Step: usn ----------
    static byte[] Le(params long[] v)
    {
        var b = new byte[v.Length * 8];
        for (int i = 0; i < v.Length; i++) BitConverter.GetBytes(v[i]).CopyTo(b, i * 8);
        return b;
    }

    static Dictionary<string, object> Query(SafeFileHandle h)
    {
        var r = new Dictionary<string, object>();
        var o = new byte[80]; int got;
        if (!DeviceIoControl(h, FSCTL_QUERY_USN_JOURNAL, null, 0, o, o.Length, out got, IntPtr.Zero))
        { r["error"] = Marshal.GetLastWin32Error(); return r; }
        r["journal_id"] = BitConverter.ToUInt64(o, 0);
        r["first_usn"] = BitConverter.ToInt64(o, 8);
        r["next_usn"] = BitConverter.ToInt64(o, 16);
        r["lowest_valid_usn"] = BitConverter.ToInt64(o, 24);
        r["max_usn"] = BitConverter.ToInt64(o, 32);
        r["maximum_size"] = BitConverter.ToUInt64(o, 40);
        r["allocation_delta"] = BitConverter.ToUInt64(o, 48);
        r["history_bytes"] = BitConverter.ToInt64(o, 16) - BitConverter.ToInt64(o, 8);
        return r;
    }

    // Reads up to maxBuffers 64 KB buffers of USN_RECORD_V2/V3 starting at startUsn.
    static Dictionary<string, object> ReadJournal(SafeFileHandle h, uint code, ulong journalId,
        long startUsn, int maxBuffers, SafeFileHandle volumeHint, int resolveMax)
    {
        var r = new Dictionary<string, object>();
        // READ_USN_JOURNAL_DATA_V1: StartUsn, ReasonMask, ReturnOnlyOnClose, Timeout,
        // BytesToWaitFor, UsnJournalID, MinMajorVersion=2, MaxMajorVersion=2 (48 bytes).
        // V2 only: its 64-bit file ids are what OpenFileById's FileIdType takes below.
        var inBuf = new byte[48];
        BitConverter.GetBytes(startUsn).CopyTo(inBuf, 0);
        BitConverter.GetBytes(0xFFFFFFFFu).CopyTo(inBuf, 8);
        BitConverter.GetBytes(journalId).CopyTo(inBuf, 32);
        BitConverter.GetBytes((ushort)2).CopyTo(inBuf, 40);
        BitConverter.GetBytes((ushort)2).CopyTo(inBuf, 42);
        var o = new byte[64 * 1024];
        long records = 0, named = 0, firstTs = 0, lastTs = 0;
        int resolved = 0, resolveFailed = 0, resolveTried = 0; double resolveMs = 0;
        var sample = new List<string>();
        var sw = Stopwatch.StartNew();
        for (int b = 0; b < maxBuffers; b++)
        {
            int got;
            if (!DeviceIoControl(h, code, inBuf, inBuf.Length, o, o.Length, out got, IntPtr.Zero))
            { r["error"] = Marshal.GetLastWin32Error(); break; }
            if (got <= 8) break;
            long next = BitConverter.ToInt64(o, 0);
            int off = 8;
            while (off < got)
            {
                int len = BitConverter.ToInt32(o, off);
                if (len <= 0) break;
                int major = BitConverter.ToUInt16(o, off + 4);
                // V2: Parent @16 (8 bytes), TimeStamp @32, Reason @40, NameLen @56, NameOff @58.
                // V3: Parent @24 (16 bytes), TimeStamp @48, Reason @56, NameLen @72, NameOff @74.
                int tsAt = major == 2 ? 32 : 48, nlAt = major == 2 ? 56 : 72;
                long ts = BitConverter.ToInt64(o, off + tsAt);
                if (firstTs == 0) firstTs = ts;
                lastTs = ts;
                int nameLen = BitConverter.ToUInt16(o, off + nlAt);
                int nameOff = BitConverter.ToUInt16(o, off + nlAt + 2);
                records++;
                if (nameLen > 0) named++;
                if (sample.Count < 5 && nameLen > 0)
                    sample.Add(Encoding.Unicode.GetString(o, off + nameOff, nameLen));
                if (volumeHint != null && resolveTried < resolveMax && major == 2)
                {
                    resolveTried++;
                    var t = Stopwatch.StartNew();
                    // FILE_ID_DESCRIPTOR: dwSize=24, Type=0 (FileIdType), FileId (8) + pad.
                    var desc = new byte[24];
                    BitConverter.GetBytes(24).CopyTo(desc, 0);
                    Array.Copy(o, off + 16, desc, 8, 8);
                    using (var p = OpenFileById(volumeHint, desc, FILE_READ_ATTRIBUTES, SHARE_ALL,
                        IntPtr.Zero, FLAG_BACKUP_SEMANTICS))
                    {
                        if (p.IsInvalid) resolveFailed++;
                        else
                        {
                            var sb = new StringBuilder(1024);
                            if (GetFinalPathNameByHandleW(p, sb, 1024, 0) > 0) resolved++;
                            else resolveFailed++;
                        }
                    }
                    resolveMs += t.Elapsed.TotalMilliseconds;
                }
                off += len;
            }
            BitConverter.GetBytes(next).CopyTo(inBuf, 0);
        }
        r["records"] = records; r["records_with_name"] = named;
        r["seconds"] = sw.Elapsed.TotalSeconds;
        if (firstTs != 0)
        {
            r["first_record_utc"] = DateTime.FromFileTimeUtc(firstTs).ToString("o");
            r["last_record_utc"] = DateTime.FromFileTimeUtc(lastTs).ToString("o");
        }
        r["sample_names"] = sample;
        r["parent_resolve_tried"] = resolveTried; r["parent_resolve_ok"] = resolved;
        r["parent_resolve_failed"] = resolveFailed;
        if (resolveTried > 0) r["parent_resolve_ms_avg"] = resolveMs / resolveTried;
        return r;
    }

    public static Dictionary<string, object> Usn(string dataRoot)
    {
        var r = new Dictionary<string, object>();
        string root = Path.GetPathRoot(dataRoot);           // "D:\"
        string vol = @"\\.\" + root.TrimEnd('\\');          // "\\.\D:"
        // A. volume handle with read access: expected ACCESS_DENIED (5) for a standard user.
        using (var v = CreateFileW(vol, GENERIC_READ, SHARE_ALL, IntPtr.Zero, OPEN_EXISTING, 0, IntPtr.Zero))
        {
            r["volume_open_read_error"] = v.IsInvalid ? Marshal.GetLastWin32Error() : 0;
            if (!v.IsInvalid) r["volume_read_query"] = Query(v);
        }
        // B. volume handle with no access rights (allowed for any user on most systems).
        using (var v = CreateFileW(vol, 0, SHARE_ALL, IntPtr.Zero, OPEN_EXISTING, 0, IntPtr.Zero))
        {
            r["volume_open_noaccess_error"] = v.IsInvalid ? Marshal.GetLastWin32Error() : 0;
            if (!v.IsInvalid) r["volume_noaccess_query"] = Query(v);
        }
        // B2. volume handle with FILE_TRAVERSE only (AssetCooker's non-admin route).
        using (var v = CreateFileW(vol, 0x20, SHARE_ALL, IntPtr.Zero, OPEN_EXISTING, 0, IntPtr.Zero))
        {
            r["volume_open_traverse_error"] = v.IsInvalid ? Marshal.GetLastWin32Error() : 0;
            if (!v.IsInvalid) r["volume_traverse_query"] = Query(v);
        }
        // B3. volume GUID path with trailing backslash, opened as a directory (BuildXL's route).
        var guid = new StringBuilder(64);
        if (GetVolumeNameForVolumeMountPointW(root, guid, 64))
        {
            r["volume_guid"] = guid.ToString();
            using (var g = CreateFileW(guid.ToString(), GENERIC_READ, SHARE_ALL, IntPtr.Zero,
                OPEN_EXISTING, FLAG_BACKUP_SEMANTICS, IntPtr.Zero))
            {
                r["guid_dir_open_error"] = g.IsInvalid ? Marshal.GetLastWin32Error() : 0;
                if (!g.IsInvalid) r["guid_dir_query"] = Query(g);
            }
        }
        // C. handle on the root directory of the volume.
        using (var d = CreateFileW(Long(root), FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES, SHARE_ALL,
            IntPtr.Zero, OPEN_EXISTING, FLAG_BACKUP_SEMANTICS, IntPtr.Zero))
        {
            if (d.IsInvalid) { r["root_open_error"] = Marshal.GetLastWin32Error(); return r; }
            var q = Query(d);
            r["root_query"] = q;
            if (q.ContainsKey("error")) return r;
            ulong id = (ulong)q["journal_id"];
            long first = (long)q["first_usn"], next = (long)q["next_usn"];
            // Oldest records: how far back the journal reaches.
            r["unprivileged_read_oldest"] = ReadJournal(d, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, id,
                first, 2, d, 200);
            // Whole journal read speed (bounded): records per second for a catch-up.
            r["unprivileged_read_bulk"] = ReadJournal(d, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, id,
                first, 4000, null, 0);
            // Privileged read on a directory handle: expected to fail without admin.
            r["privileged_read_on_dir"] = ReadJournal(d, FSCTL_READ_USN_JOURNAL, id, first, 1, null, 0);
            // MFT enumeration (MFT_ENUM_DATA_V0: StartFileReferenceNumber, LowUsn, HighUsn).
            var o = new byte[64 * 1024]; int got;
            bool ok = DeviceIoControl(d, FSCTL_ENUM_USN_DATA, Le(0, 0, next), 24, o, o.Length, out got, IntPtr.Zero);
            r["enum_usn_data_on_dir"] = ok ? (object)("ok, bytes=" + got) : Marshal.GetLastWin32Error();
        }
        return r;
    }

    public static Dictionary<string, object> UsnQueryOnly(string dataRoot)
    {
        string root = Path.GetPathRoot(dataRoot);
        using (var d = CreateFileW(Long(root), FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES, SHARE_ALL,
            IntPtr.Zero, OPEN_EXISTING, FLAG_BACKUP_SEMANTICS, IntPtr.Zero))
        {
            if (d.IsInvalid) { var e = new Dictionary<string, object>(); e["error"] = Marshal.GetLastWin32Error(); return e; }
            var q = Query(d);
            q["taken_utc"] = DateTime.UtcNow.ToString("o");
            return q;
        }
    }

    // ---------- Step: semantics (directory mtime, stale sizes) ----------
    static long ListedWriteTime(string parent, string name, out long listedSize)
    {
        listedSize = -1;
        WIN32_FIND_DATAW d;
        IntPtr h = FindFirstFileExW(Long(parent) + "\\" + name, FindExInfoBasic, out d,
            FindExSearchNameMatch, IntPtr.Zero, 0);
        if (h == new IntPtr(-1)) return -1;
        FindClose(h);
        listedSize = ((long)d.nFileSizeHigh << 32) | d.nFileSizeLow;
        return ((long)d.ftLastWriteHigh << 32) | d.ftLastWriteLow;
    }

    public static List<string> Semantics(string scratch)
    {
        var o = new List<string>();
        Directory.CreateDirectory(scratch);
        string t = Path.Combine(scratch, "T");
        if (Directory.Exists(t)) Directory.Delete(t, true);
        Directory.CreateDirectory(t);
        File.WriteAllText(Path.Combine(t, "existing.txt"), "x");
        Directory.CreateDirectory(Path.Combine(t, "sub"));
        long sz;
        // "listed" is T's mtime as its parent's listing reports it (the copy NTFS keeps in the
        // parent's index, which it may update lazily); "own" is T's own mtime, read through the
        // directory itself. The runner rehearsal (VFS-5) found the two can differ.
        Action<string, Action> probe = (label, act) =>
        {
            long before = ListedWriteTime(scratch, "T", out sz);
            long ownBefore = Directory.GetLastWriteTimeUtc(t).ToFileTimeUtc();
            Thread.Sleep(1500);
            act();
            long after0 = ListedWriteTime(scratch, "T", out sz);
            long own0 = Directory.GetLastWriteTimeUtc(t).ToFileTimeUtc();
            Thread.Sleep(3000);
            long after3 = ListedWriteTime(scratch, "T", out sz);
            long own3 = Directory.GetLastWriteTimeUtc(t).ToFileTimeUtc();
            o.Add(string.Format("{0}: listed dir mtime changed immediately={1}, after 3 s={2}; " +
                "own dir mtime changed immediately={3}, after 3 s={4}",
                label, after0 != before, after3 != before, own0 != ownBefore, own3 != ownBefore));
        };
        probe("create file in T", () => File.WriteAllText(Path.Combine(t, "new.txt"), "x"));
        probe("append to existing file in T", () => File.AppendAllText(Path.Combine(t, "existing.txt"), "more"));
        probe("rename file in T", () => File.Move(Path.Combine(t, "new.txt"), Path.Combine(t, "renamed.txt")));
        probe("create file in T\\sub (grandchild)", () => File.WriteAllText(Path.Combine(t, "sub", "g.txt"), "x"));
        probe("delete file in T", () => File.Delete(Path.Combine(t, "renamed.txt")));
        // Stale size in the directory entry while a writer keeps the file open.
        string f = Path.Combine(t, "growing.bin");
        using (var fs = new FileStream(f, FileMode.Create, FileAccess.Write, FileShare.ReadWrite))
        {
            fs.Write(new byte[1 << 20], 0, 1 << 20);
            fs.Flush(true);
            long listed; ListedWriteTime(t, "growing.bin", out listed);
            o.Add(string.Format("open writer, 1 MiB written: listed size={0}, real size={1}", listed, fs.Length));
        }
        long closed; ListedWriteTime(t, "growing.bin", out closed);
        o.Add(string.Format("after close: listed size={0}", closed));
        Directory.Delete(t, true);
        return o;
    }

    // ---------- Step: flush (cost of FlushFileBuffers, i.e. SQLite commits/checkpoints) ----------
    public static Dictionary<string, object> Flush(string dir, int rounds)
    {
        Directory.CreateDirectory(dir);
        string f = Path.Combine(dir, "flush-test.bin");
        var r = new Dictionary<string, object>();
        var small = new List<double>(); var big = new List<double>();
        using (var fs = new FileStream(f, FileMode.Create, FileAccess.ReadWrite, FileShare.None, 4096))
        {
            var b4 = new byte[4096]; var b1m = new byte[1 << 20];
            for (int i = 0; i < rounds; i++)
            {
                var sw = Stopwatch.StartNew();
                fs.Write(b4, 0, b4.Length); fs.Flush(true);
                small.Add(sw.Elapsed.TotalMilliseconds);
            }
            for (int i = 0; i < rounds / 4 + 1; i++)
            {
                var sw = Stopwatch.StartNew();
                fs.Write(b1m, 0, b1m.Length); fs.Flush(true);
                big.Add(sw.Elapsed.TotalMilliseconds);
            }
        }
        File.Delete(f);
        small.Sort(); big.Sort();
        r["flush_4k_ms_p50"] = small[small.Count / 2]; r["flush_4k_ms_p95"] = small[(int)(small.Count * 0.95)];
        r["flush_1m_ms_p50"] = big[big.Count / 2]; r["flush_1m_ms_max"] = big[big.Count - 1];
        return r;
    }

    // ---------- Step: seek (random 4 KiB unbuffered reads, normal vs very-low I/O priority) ----------
    public static Dictionary<string, object> Seek(string dir, int sizeMb, int samples)
    {
        Directory.CreateDirectory(dir);
        string f = Path.Combine(dir, "seek-test.bin");
        var r = new Dictionary<string, object>();
        var chunk = new byte[1 << 20];
        new Random(1).NextBytes(chunk);
        var w = Stopwatch.StartNew();
        using (var fs = new FileStream(f, FileMode.Create, FileAccess.Write, FileShare.None, 1 << 20))
        {
            for (int i = 0; i < sizeMb; i++) fs.Write(chunk, 0, chunk.Length);
            fs.Flush(true);
        }
        r["write_mb_per_sec"] = sizeMb / w.Elapsed.TotalSeconds;
        foreach (int prio in new int[] { 2, 0 }) // IoPriorityHintNormal=2, VeryLow=0
        {
            var lat = new List<double>();
            using (var h = CreateFileW(f, GENERIC_READ, SHARE_ALL, IntPtr.Zero, OPEN_EXISTING, FLAG_NO_BUFFERING, IntPtr.Zero))
            {
                if (h.IsInvalid) { r["open_error"] = Marshal.GetLastWin32Error(); break; }
                int p = prio;
                bool set = SetFileInformationByHandle(h, FileIoPriorityHintInfo, ref p, 4);
                r["priority_" + prio + "_set"] = set;
                // Unbuffered I/O needs sector-aligned offsets, sizes and buffer addresses;
                // VirtualAlloc returns page-aligned memory and 4096 covers 512e and 4Kn disks.
                IntPtr buf = VirtualAlloc(IntPtr.Zero, (UIntPtr)4096, 0x3000, 0x04);
                var rnd = new Random(prio + 7);
                long blocks = (long)sizeMb * 256;
                for (int i = 0; i < samples; i++)
                {
                    long blk = (long)(rnd.NextDouble() * (blocks - 1));
                    int read;
                    var sw = Stopwatch.StartNew();
                    long ignored;
                    SetFilePointerEx(h, blk * 4096, out ignored, 0);
                    if (!ReadFile(h, buf, 4096, out read, IntPtr.Zero)) { r["read_error"] = Marshal.GetLastWin32Error(); break; }
                    lat.Add(sw.Elapsed.TotalMilliseconds);
                }
                VirtualFree(buf, UIntPtr.Zero, 0x8000);
            }
            if (lat.Count == 0) continue;
            lat.Sort();
            string k = prio == 2 ? "normal" : "verylow";
            r["read4k_" + k + "_ms_p50"] = lat[lat.Count / 2];
            r["read4k_" + k + "_ms_p95"] = lat[(int)(lat.Count * 0.95)];
            r["read4k_" + k + "_ms_max"] = lat[lat.Count - 1];
        }
        File.Delete(f);
        return r;
    }

    // ---------- Step: watch (ReadDirectoryChangesW event rate and overflows) ----------
    public class Counter
    {
        public long Events, Errors;
        public FileSystemWatcher W;
        public Counter(string root, NotifyFilters filter)
        {
            W = new FileSystemWatcher(root);
            W.IncludeSubdirectories = true;
            W.InternalBufferSize = 64 * 1024;
            W.NotifyFilter = filter;
            FileSystemEventHandler h = (s, e) => Interlocked.Increment(ref Events);
            W.Created += h; W.Changed += h; W.Deleted += h;
            W.Renamed += (s, e) => Interlocked.Increment(ref Events);
            W.Error += (s, e) => Interlocked.Increment(ref Errors);
            W.EnableRaisingEvents = true;
        }
    }

    public static Counter WatchAsVfs(string root)
    {
        // The filter VFS uses today (indexer/watch.rs FILTER).
        return new Counter(root, NotifyFilters.FileName | NotifyFilters.DirectoryName |
            NotifyFilters.Size | NotifyFilters.LastWrite | NotifyFilters.CreationTime);
    }

    public static Counter WatchNamesOnly(string root)
    {
        return new Counter(root, NotifyFilters.FileName | NotifyFilters.DirectoryName);
    }
}
'@

if (-not ('VfsProbe' -as [type])) { Add-Type -TypeDefinition $cs -Language CSharp }

$all = $Step -eq 'all'

if ($all -or $Step -eq 'info') {
  Try-Step 'info' {
    $info = [ordered]@{}
    $os = Get-CimInstance Win32_OperatingSystem
    $info.os = "$($os.Caption) $($os.Version) build $($os.BuildNumber)"
    $info.ram_total_gb = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
    $info.ram_free_gb = [math]::Round($os.FreePhysicalMemory / 1MB, 2)
    $info.commit_used_gb = [math]::Round(($os.TotalVirtualMemorySize - $os.FreeVirtualMemory) / 1MB, 2)
    $info.cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1 Name, NumberOfCores, NumberOfLogicalProcessors, MaxClockSpeed)
    $info.disks = @(Get-CimInstance Win32_DiskDrive | Select-Object Index, Model, InterfaceType, MediaType, @{n = 'SizeGB'; e = { [math]::Round($_.Size / 1GB) } })
    try { $info.physical_disks = @(Get-PhysicalDisk | Select-Object FriendlyName, MediaType, BusType, SpindleSpeed, HealthStatus, @{n = 'SizeGB'; e = { [math]::Round($_.Size / 1GB) } }) } catch { $info.physical_disks = "error: $($_.Exception.Message)" }
    $info.volumes = @(Get-CimInstance Win32_Volume | Where-Object DriveLetter | Select-Object DriveLetter, FileSystem, BlockSize, IndexingEnabled, @{n = 'CapacityGB'; e = { [math]::Round($_.Capacity / 1GB) } }, @{n = 'FreeGB'; e = { [math]::Round($_.FreeSpace / 1GB) } })
    $info.letter_to_disk = @(Get-CimInstance Win32_LogicalDisk | ForEach-Object {
        $ld = $_
        $parts = Get-CimAssociatedInstance -InputObject $ld -ResultClassName Win32_DiskPartition -ErrorAction SilentlyContinue
        foreach ($p in $parts) { [pscustomobject]@{ Letter = $ld.DeviceID; Disk = $p.DiskIndex; Partition = $p.Index } }
      })
    $info.localappdata = $env:LOCALAPPDATA
    $info.appdata = $env:APPDATA
    $info.data_root = $DataRoot
    $info.long_paths_enabled = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' -ErrorAction SilentlyContinue).LongPathsEnabled
    $info.ntfs_disable_8dot3 = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' -ErrorAction SilentlyContinue).NtfsDisable8dot3NameCreation
    $info.ntfs_disable_last_access = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' -ErrorAction SilentlyContinue).NtfsDisableLastAccessUpdate
    $wv = 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    $info.webview2_machine = (Get-ItemProperty "HKLM:\$wv" -ErrorAction SilentlyContinue).pv
    $info.webview2_user = (Get-ItemProperty 'HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue).pv
    $info.is_admin_token = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    $info.user_in_admin_group = [bool](whoami /groups | Select-String 'S-1-5-32-544')
    $info.power_scheme = (powercfg /getactivescheme) -join ' '
    try { $info.defender = Get-MpComputerStatus | Select-Object AMRunningMode, RealTimeProtectionEnabled, OnAccessProtectionEnabled, IoavProtectionEnabled, AntivirusSignatureLastUpdated, FullScanAge, QuickScanAge } catch { $info.defender = "error: $($_.Exception.Message)" }
    $info.other_av = @(Get-CimInstance -Namespace root/SecurityCenter2 -ClassName AntiVirusProduct -ErrorAction SilentlyContinue | Select-Object displayName, productState)
    $info.sync_processes = @(Get-Process | Where-Object { $_.Path -match 'Synology|Drive|OneDrive|Dropbox|Google' -or $_.ProcessName -match 'cloud-drive|synology|onedrive' } |
      Select-Object ProcessName, Path, @{n = 'WorkingSetMB'; e = { [math]::Round($_.WorkingSet64 / 1MB) } }, @{n = 'PrivateMB'; e = { [math]::Round($_.PrivateMemorySize64 / 1MB) } })
    # A standard user may be refused the Service Control Manager (seen on Windows Server 2025).
    try { $info.sync_services = @(Get-Service -ErrorAction Stop | Where-Object { $_.DisplayName -match 'Synology|Active Backup|OneDrive' } | Select-Object Name, DisplayName, Status, StartType) } catch { $info.sync_services = "error: $($_.Exception.Message)" }
    $info.top_memory = @(Get-Process | Sort-Object WorkingSet64 -Descending | Select-Object -First 15 ProcessName, @{n = 'WorkingSetMB'; e = { [math]::Round($_.WorkingSet64 / 1MB) } }, @{n = 'PrivateMB'; e = { [math]::Round($_.PrivateMemorySize64 / 1MB) } })
    $info.fsutil_8dot3 = (fsutil 8dot3name query ([IO.Path]::GetPathRoot($DataRoot).TrimEnd('\')) 2>&1) -join ' | '
    $info.fsutil_lastaccess = (fsutil behavior query disablelastaccess 2>&1) -join ' | '
    $info.fsutil_usn_admin_only = (fsutil usn queryjournal ([IO.Path]::GetPathRoot($DataRoot).TrimEnd('\')) 2>&1) -join ' | '
    Save 'info' $info
  }
}

if ($all -or $Step -eq 'usn') {
  Try-Step 'usn' {
    Save 'usn' ([VfsProbe]::Usn($DataRoot))
    # Journal growth rate: a second query later in the visit gives bytes/hour (see the plan).
    Save 'usn-query-1' ([VfsProbe]::UsnQueryOnly($DataRoot))
  }
}

if ($all -or $Step -eq 'walk') {
  Try-Step 'walk' {
    $secs = $WalkMinutes * 60
    Log "walk 1/4: FindFirstFileEx basic, cold cache, background mode toggled every 2000 dirs"
    $w = [ordered]@{}
    $w.cold_basic_background = [VfsProbe]::Walk($DataRoot, 0, $true, $MaxDirs, $secs, 2000)
    Log ("  {0} dirs, {1} files in {2:N0} s" -f $w.cold_basic_background.dirs, $w.cold_basic_background.files, $w.cold_basic_background.seconds)
    $n = [int]$w.cold_basic_background.dirs
    Log "walk 2/4: same directories again (warm cache)"
    $w.warm_basic = [VfsProbe]::Walk($DataRoot, 0, $false, $n, $secs, 0)
    Log "walk 3/4: warm, FIND_FIRST_EX_LARGE_FETCH"
    $w.warm_large_fetch = [VfsProbe]::Walk($DataRoot, 1, $false, $n, $secs, 0)
    Log "walk 4/4: warm, GetFileInformationByHandleEx(FileIdBothDirectoryInfo)"
    $w.warm_by_handle = [VfsProbe]::Walk($DataRoot, 2, $false, $n, $secs, 0)
    Save 'walk' $w
  }
}

if ($all -or $Step -eq 'semantics') {
  Try-Step 'semantics' { Save 'semantics' ([VfsProbe]::Semantics((New-Scratch))) }
}

if ($all -or $Step -eq 'flush') {
  Try-Step 'flush' {
    $f = [ordered]@{}
    $f.localappdata = [VfsProbe]::Flush((Join-Path $run 'scratch'), 40)
    $f.data_disk = [VfsProbe]::Flush((New-Scratch), 40)
    Save 'flush' $f
  }
}

if ($all -or $Step -eq 'seek') {
  Try-Step 'seek' {
    $free = (Get-CimInstance Win32_LogicalDisk -Filter ("DeviceID='{0}'" -f [IO.Path]::GetPathRoot($ScratchDir).TrimEnd('\'))).FreeSpace
    if ($free -lt 20GB) { Log "seek skipped: less than 20 GB free" }
    else { Save 'seek' ([VfsProbe]::Seek((New-Scratch), 2048, 400)) }
  }
}

if ($all -or $Step -eq 'watch' -or $Step -eq 'perf') {
  Try-Step 'watch+perf' {
    $vfs = $null; $names = $null
    if ($all -or $Step -eq 'watch') {
      $vfs = [VfsProbe]::WatchAsVfs($DataRoot)
      $names = [VfsProbe]::WatchNamesOnly($DataRoot)
    }
    $csv = Join-Path $run 'perf.csv'
    'time,disk,idle_pct,cur_queue,avg_queue,read_ms,write_ms,reads_s,writes_s,avail_mb,pages_s,events_vfs,events_names,overflows_vfs,overflows_names,top_io' | Set-Content $csv -Encoding UTF8
    $end = (Get-Date).AddMinutes($Minutes)
    $prevRaw = @{}
    Log "sampling disk, memory and watcher events every 5 s for $Minutes min; use the PC normally"
    while ((Get-Date) -lt $end) {
      $t = Get-Date -Format 'HH:mm:ss'
      $mem = Get-CimInstance Win32_PerfFormattedData_PerfOS_Memory
      $top = (Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $_.Name -ne '_Total' -and $_.Name -ne 'Idle' } |
          Sort-Object IODataBytesPersec -Descending | Select-Object -First 3 | ForEach-Object { '{0}:{1}KB/s' -f $_.Name, [math]::Round($_.IODataBytesPersec / 1KB) }) -join ' '
      $ev1 = if ($vfs) { $vfs.Events } else { '' }; $ev2 = if ($names) { $names.Events } else { '' }
      $er1 = if ($vfs) { $vfs.Errors } else { '' }; $er2 = if ($names) { $names.Errors } else { '' }
      # Latency from the raw class: the formatted AvgDisksecPerRead is an integer in seconds.
      $raw = @{}
      foreach ($r in (Get-CimInstance Win32_PerfRawData_PerfDisk_PhysicalDisk | Where-Object Name -ne '_Total')) { $raw[$r.Name] = $r }
      $disks = @(Get-CimInstance Win32_PerfFormattedData_PerfDisk_PhysicalDisk | Where-Object Name -ne '_Total')
      # The runner rehearsal (VFS-5) got no rows here as a standard user: say so in the log.
      if (-not $disks.Count -and -not $script:noDisksLogged) { Log 'perf: no PhysicalDisk counter instances visible to this user'; $script:noDisksLogged = $true }
      foreach ($d in $disks) {
        $readMs = ''; $writeMs = ''
        $r = $raw[$d.Name]; $p = $prevRaw[$d.Name]
        if ($r -and $p) {
          $f = [double]$r.Frequency_PerfTime
          $nb = [double]$r.AvgDisksecPerRead_Base - $p.AvgDisksecPerRead_Base
          if ($nb -gt 0) { $readMs = [math]::Round(1000 * ([double]$r.AvgDisksecPerRead - $p.AvgDisksecPerRead) / $f / $nb, 2) }
          $nb = [double]$r.AvgDisksecPerWrite_Base - $p.AvgDisksecPerWrite_Base
          if ($nb -gt 0) { $writeMs = [math]::Round(1000 * ([double]$r.AvgDisksecPerWrite - $p.AvgDisksecPerWrite) / $f / $nb, 2) }
        }
        '{0},"{1}",{2},{3},{4},{5},{6},{7},{8},{9},{10},{11},{12},{13},{14},"{15}"' -f $t, $d.Name, $d.PercentIdleTime, $d.CurrentDiskQueueLength,
        $d.AvgDiskQueueLength, $readMs, $writeMs, $d.DiskReadsPersec, $d.DiskWritesPersec,
        $mem.AvailableMBytes, $mem.PagesPersec, $ev1, $ev2, $er1, $er2, $top | Add-Content $csv -Encoding UTF8
      }
      $prevRaw = $raw
      Start-Sleep -Seconds 5
    }
    if ($vfs) {
      Save 'watch' ([ordered]@{ minutes = $Minutes; vfs_filter_events = $vfs.Events; vfs_filter_overflows = $vfs.Errors; names_only_events = $names.Events; names_only_overflows = $names.Errors })
      $vfs.W.Dispose(); $names.W.Dispose()
    }
  }
}

if ($all -or $Step -eq 'usn-query') {
  # End-of-visit query only (no reads): compare with usn-query-1.json for bytes per hour.
  Try-Step 'usn-query' { Save 'usn-query-2' ([VfsProbe]::UsnQueryOnly($DataRoot)) }
}

# Delete only the folder this run created.
if ($scratchCreated) { Remove-Item -LiteralPath $ScratchDir -Recurse -Force -ErrorAction SilentlyContinue }
Log "done. Results in $run"
